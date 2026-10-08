(() => {
  const checks = [], resources = [];
  const check = (name, actual, expected) => checks.push({
    name, actual, expected, passed: JSON.stringify(actual) === JSON.stringify(expected),
  });
  const frame = document.createElement('iframe');
  document.body.appendChild(frame); resources.push(frame);
  let serial = 0;
  try {
    const realms = [['main', window], ['iframe', frame.contentWindow]];
    for (const [sourceName, sourceWindow] of realms) {
      for (const [targetName, targetWindow] of realms) {
        const sourceDocument = sourceWindow.document, targetDocument = targetWindow.document;
        const registryGetter = Object.getOwnPropertyDescriptor(sourceWindow.Element.prototype, 'customElementRegistry').get;
        for (const shadowKind of ['implicit', 'null', 'global', 'scoped']) {
          for (const childKind of ['null', 'global', 'scoped']) {
            for (const selfOnly of [false, true]) {
              const tag = `x-clone-registry-${serial++}`;
              const scoped = new sourceWindow.CustomElementRegistry();
              const fallback = new targetWindow.CustomElementRegistry();
              class SourceGlobal extends sourceWindow.HTMLElement { constructor() { super(); this.marker = 'source-global'; } }
              class SourceScoped extends sourceWindow.HTMLElement { constructor() { super(); this.marker = 'source-scoped'; } }
              class TargetGlobal extends targetWindow.HTMLElement { constructor() { super(); this.marker = 'target-global'; } }
              class Fallback extends targetWindow.HTMLElement { constructor() { super(); this.marker = 'fallback'; } }
              sourceWindow.customElements.define(tag, SourceGlobal);
              scoped.define(tag, SourceScoped); fallback.define(tag, Fallback);
              if (sourceWindow !== targetWindow) targetWindow.customElements.define(tag, TargetGlobal);
              const source = sourceDocument.createElement('div', {customElementRegistry: null});
              const options = {mode: 'open', clonable: true, serializable: true};
              if (shadowKind !== 'implicit') options.customElementRegistry = shadowKind === 'null' ? null : shadowKind === 'global' ? sourceWindow.customElements : scoped;
              const shadow = source.attachShadow(options);
              const make = kind => {
                const registry = kind === 'null' ? null : kind === 'global' ? sourceWindow.customElements : scoped;
                const child = sourceDocument.createElement(tag, {customElementRegistry: registry});
                child.textContent = '\ud800 value\udfff\r';
                child.setAttribute('data-value', '\udfff attribute\ud800');
                return child;
              };
              const light = make(childKind), shadowChild = make(childKind);
              source.appendChild(light); shadow.appendChild(shadowChild);
              const nestedHost = sourceDocument.createElement('div', {customElementRegistry: null});
              shadow.appendChild(nestedHost);
              const nestedRoot = nestedHost.attachShadow({mode: 'open', clonable: true, customElementRegistry: sourceWindow.customElements});
              const nestedNull = make('null'), nestedScoped = make('scoped');
              nestedRoot.append(nestedNull, nestedScoped);
              const sourceRegistry = registryGetter.call(shadowChild);
              const sourceLightRegistry = registryGetter.call(light);
              const sourceRootRegistry = shadow.customElementRegistry;
              const key = registry => registry === null ? 'null' : registry === scoped ? 'scoped' : registry === fallback ? 'fallback' : registry === sourceWindow.customElements ? 'source-global' : registry === targetWindow.customElements ? 'target-global' : String(registry);
              const rebaseGlobal = registry => registry === sourceWindow.customElements ? targetWindow.customElements : registry;
              const constructorFor = registry => registry === null ? null : registry.get(tag);
              const poison = childKind === 'scoped' && selfOnly;
              if (poison) {
                for (const object of [source, shadow, shadowChild, nestedHost]) {
                  for (const property of ['customElementRegistry', 'ownerDocument', 'childNodes', 'firstChild', 'shadowRoot']) {
                    Object.defineProperty(object, property, {configurable: true, get() { throw Error(`author ${property} getter`); }});
                  }
                }
              }
              const operations = [
                ['import-default', {selfOnly}, targetWindow.customElements],
                ['import-fallback', {selfOnly, customElementRegistry: fallback}, fallback],
              ];
              if (sourceWindow === targetWindow) operations.push(['clone', !selfOnly, null]);
              for (const [method, argument, lightFallback] of operations) {
                const prefix = `${sourceName}->${targetName}/${shadowKind}/${childKind}/${selfOnly ? 'self' : 'deep'}/${method}`;
                let clone;
                try {
                  clone = method === 'clone' ? sourceWindow.Node.prototype.cloneNode.call(source, argument)
                    : targetWindow.Document.prototype.importNode.call(targetDocument, source, argument);
                } catch (error) {
                  check(`${prefix}/operation succeeds`, String(error), 'success');
                  continue;
                }
                check(`${prefix}/root`, [clone !== source, clone.ownerDocument === targetDocument, clone.parentNode, key(clone.customElementRegistry)], [true, true, null, key(lightFallback)]);
                check(`${prefix}/light children`, clone.childNodes.length, selfOnly ? 0 : 1);
                const cloneRoot = clone.shadowRoot;
                check(`${prefix}/shadow metadata`, [cloneRoot !== shadow, cloneRoot.host === clone, cloneRoot.ownerDocument === targetDocument, cloneRoot.clonable, cloneRoot.serializable, key(cloneRoot.customElementRegistry)], [true, true, true, true, true, key(rebaseGlobal(sourceRootRegistry))]);
                const element = (label, node, registry) => {
                  const Constructor = constructorFor(registry);
                  check(`${prefix}/${label} registry`, key(node.customElementRegistry), key(registry));
                  check(`${prefix}/${label} constructor`, [node.marker ?? null, Object.getPrototypeOf(node) === (Constructor ? Constructor.prototype : targetWindow.HTMLElement.prototype)], [Constructor === SourceScoped ? 'source-scoped' : Constructor === Fallback ? 'fallback' : Constructor === SourceGlobal ? 'source-global' : Constructor === TargetGlobal ? 'target-global' : null, true]);
                  check(`${prefix}/${label} native data`, [node.ownerDocument === targetDocument, node.textContent, node.getAttribute('data-value')], [true, '\ud800 value\udfff\r', '\udfff attribute\ud800']);
                };
                if (!selfOnly) element('light child', clone.firstChild, sourceLightRegistry === null ? lightFallback : rebaseGlobal(sourceLightRegistry));
                element('shadow child', cloneRoot.firstChild, rebaseGlobal(sourceRegistry));
                const nested = cloneRoot.lastChild.shadowRoot;
                check(`${prefix}/nested shadow registry`, key(nested.customElementRegistry), key(targetWindow.customElements));
                element('nested null child', nested.firstChild, null);
                element('nested scoped child', nested.lastChild, scoped);
                check(`${prefix}/source registry unchanged`, key(registryGetter.call(shadowChild)), key(sourceRegistry));
                check(`${prefix}/source native data unchanged`, [light.ownerDocument === sourceDocument, nestedScoped.ownerDocument === sourceDocument], [true, true]);
              }
            }
          }
        }
      }
    }
    globalThis.__documentCloneRegistryResults = {complete: true, total: checks.length, passed: checks.filter(row => row.passed).length, checks};
    globalThis.__uiEventResults = globalThis.__documentCloneRegistryResults;
    return true;
  } finally { for (const resource of resources.reverse()) resource.remove(); }
})()
