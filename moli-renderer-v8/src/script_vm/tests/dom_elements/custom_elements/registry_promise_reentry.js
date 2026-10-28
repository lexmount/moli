(() => {
  const registry = new CustomElementRegistry();
  const pending = registry.whenDefined('x-resolving');
  class Resolving extends HTMLElement {}
  let reads = 0;
  const observations = [];
  Object.defineProperty(Resolving, 'then', {get() {
    reads++;
    observations.push(registry.get('x-resolving') === Resolving);
    const name = 'x-reentrant-' + reads;
    const Nested = class extends HTMLElement {};
    registry.define(name, Nested);
    observations.push(registry.get(name) === Nested);
    registry.whenDefined(name);
    return undefined;
  }});
  registry.define('x-resolving', Resolving);
  const ready = registry.whenDefined('x-resolving');
  let fulfilled = 0;
  pending.then(value => { if (value === Resolving) fulfilled++; });
  ready.then(value => { if (value === Resolving) fulfilled++; });
  globalThis.registryPromiseReentryFacts = () =>
    JSON.stringify({reads, observations, fulfilled});
  return 'ready';
})()
