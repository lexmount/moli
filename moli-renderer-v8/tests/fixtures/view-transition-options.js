(() => {
  const assert = (value, label) => { if (!value) throw Error(label); };
  const keep = transition => {
    transition.ready.catch(() => {}); transition.finished.catch(() => {});
    transition.updateCallbackDone.catch(() => {});
    return transition;
  };
  const members = ['types', 'update'];
  const active = keep(document.startViewTransition(() => {}));
  for (const member of members) for (const sentinel of [{}, null, undefined, 7]) {
    const reads = []; let caught = Symbol();
    try {document.startViewTransition(new Proxy({}, {get(_, key) {
      reads.push(key); if(key === member) throw sentinel; return undefined;
    }, has() {throw Error('Get required');}}));} catch(e) {caught=e;}
    assert(caught === sentinel && reads.join() === members.slice(0, members.indexOf(member)+1).join(), 'original exception and order');
    assert(document.activeViewTransition === active, 'failed conversion must preserve active transition');
  }
  for (const value of [1, true, 'init', 1n, Symbol()]) {
    let caught; try {document.startViewTransition(value);} catch(e) {caught=e;}
    assert(caught instanceof TypeError && document.activeViewTransition === active, 'primitive union rejected without mutation');
  }
  const sentinel = {}, types = {[Symbol.iterator]() {return {next() {throw sentinel;}};}};
  let reads = 0, caught;
  try {document.startViewTransition({types, get update() {reads++;return null;}});} catch(e) {caught=e;}
  assert(caught === sentinel && reads === 0 && document.activeViewTransition === active, 'sequence conversion before update and mutation');
  active.skipTransition();
  for (const types of [undefined, null, []]) {
    const t = keep(document.startViewTransition({types, update:null}));
    assert(t.types.size === 0, 'legacy nullish sequence default'); t.skipTransition();
  }
  const read = [];
  const transition = keep(document.startViewTransition(new Proxy({}, {get(_, key) {
    read.push(key); return key === 'types' ? new Set(['one', 'two']) : null;
  }})));
  assert(read.join() === 'types,update' && transition.types.has('one') && transition.types.has('two'), 'full derived dictionary');
  transition.skipTransition();
  const callback = new Proxy(() => {}, {get(_, key) {if(members.includes(key)) throw Error('callable is not dictionary');}});
  const callable = keep(document.startViewTransition(callback)); callable.skipTransition();
  return true;
})()
