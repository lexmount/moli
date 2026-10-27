function getHtmlOptionsProbe(variant) {
  const assert = (value, message) => { if (!value) throw new Error(message); };
  const frame = document.querySelector('iframe');
  assert(frame, 'getHTML probe requires a child realm');
  const other = frame.contentWindow;
  assert(other.Object !== Object && other.TypeError !== TypeError, 'distinct child realm intrinsics');
  const groups = {
    dictionary: ['omitted','undefined','null','number','string','boolean','bigint','symbol','serializable','array','dictionary-order'],
    iterables: ['set','array-like','sequence-null','sequence-number','invalid-element','forged-root','author-proxy','revoked-proxy','iterable','invalid-iterator-item','iterable-length-throws'],
    exceptions: ['iterator-getter-throws','iterator-next-throws','serializable-getter-throws','roots-getter-throws','array-first-getter-throws'],
    mutation: ['iterator-mutation']
  };
  const cases = groups[variant];
  assert(cases, 'unknown getHTML probe variant');
  const typeErrors = new Set(['number','string','boolean','bigint','symbol','array-like','sequence-null','sequence-number','invalid-element','forged-root','author-proxy','revoked-proxy','invalid-iterator-item']);
  const sentinels = new Set(groups.exceptions);
  const fullRoots = new Set(['array','set','iterable','iterable-length-throws','dictionary-order','iterator-mutation']);
  const expectedLogs = {
    iterable: ['iterator','next:0','next:1','next:2'],
    'invalid-iterator-item': ['iterator','next'],
    'iterable-length-throws': ['iterator'],
    'iterator-getter-throws': ['iterator-get'],
    'iterator-next-throws': ['iterator','next'],
    'serializable-getter-throws': ['serializable'],
    'roots-getter-throws': ['serializable','roots'],
    'array-first-getter-throws': ['index:0'],
    'dictionary-order': ['serializableShadowRoots','shadowRoots'],
    'iterator-mutation': ['serializable','roots','iterator']
  };
  const light = '<b>light</b>';
  const plain = '<i>shadow</i><section></section>';
  const complete = '<i>shadow</i><section><template shadowrootmode="closed"><em>nested</em></template></section>';
  const outerStart = '<template shadowrootmode="open" shadowrootserializable="">';
  function optionsFor(id, root, nested, host, log, sentinel) {
    const roots = [root,nested];
    switch (id) {
      case 'omitted': return [];
      case 'undefined': return [undefined];
      case 'null': return [null];
      case 'number': return [1];
      case 'string': return ['options'];
      case 'boolean': return [true];
      case 'bigint': return [1n];
      case 'symbol': return [Symbol('options')];
      case 'serializable': return [{serializableShadowRoots:true}];
      case 'array': return [{shadowRoots:roots}];
      case 'set': return [{shadowRoots:new Set(roots)}];
      case 'array-like': return [{shadowRoots:{0:root,1:nested,length:2}}];
      case 'sequence-null': return [{shadowRoots:null}];
      case 'sequence-number': return [{shadowRoots:1}];
      case 'invalid-element': return [{shadowRoots:[host]}];
      case 'forged-root': return [{shadowRoots:[Object.create(root)]}];
      case 'author-proxy': return [{shadowRoots:[new Proxy(root,{get(){log.push('author-get');throw sentinel;}})]}];
      case 'revoked-proxy': {
        const {proxy,revoke} = Proxy.revocable(root,{});
        revoke();
        return [{shadowRoots:[proxy]}];
      }
      case 'iterable': return [{shadowRoots:{[Symbol.iterator](){
        log.push('iterator');let i=0;
        return {next(){log.push('next:'+i);return i<roots.length?{value:roots[i++],done:false}:{done:true};}};
      }}}];
      case 'iterator-getter-throws': return [{shadowRoots:{get [Symbol.iterator](){log.push('iterator-get');throw sentinel;}}}];
      case 'iterator-next-throws': return [{shadowRoots:{[Symbol.iterator](){
        log.push('iterator');return {next(){log.push('next');throw sentinel;},return(){log.push('return');return {};}};
      }}}];
      case 'invalid-iterator-item': return [{shadowRoots:{[Symbol.iterator](){
        log.push('iterator');return {next(){log.push('next');return {value:host,done:false};},return(){log.push('return');return {};}};
      }}}];
      case 'serializable-getter-throws': return [{
        get serializableShadowRoots(){log.push('serializable');throw sentinel;},
        get shadowRoots(){log.push('roots');return roots;}
      }];
      case 'roots-getter-throws': return [{
        get serializableShadowRoots(){log.push('serializable');return false;},
        get shadowRoots(){log.push('roots');throw sentinel;}
      }];
      case 'iterable-length-throws': return [{shadowRoots:{
        get length(){log.push('length');throw sentinel;},
        [Symbol.iterator](){log.push('iterator');return roots[Symbol.iterator]();}
      }}];
      case 'array-first-getter-throws': {
        const value = new Array(2);
        Object.defineProperty(value,'0',{get(){log.push('index:0');throw sentinel;}});
        Object.defineProperty(value,'1',{get(){log.push('index:1');return nested;}});
        return [{shadowRoots:value}];
      }
      case 'dictionary-order': return [new Proxy({serializableShadowRoots:false,shadowRoots:new Set(roots)},{
        get(target,key,receiver){log.push(String(key));return Reflect.get(target,key,receiver);}
      })];
      case 'iterator-mutation': return [{
        get serializableShadowRoots(){log.push('serializable');root.firstChild.textContent='changed';return false;},
        get shadowRoots(){log.push('roots');return {[Symbol.iterator](){
          log.push('iterator');nested.firstChild.textContent='mutated';return roots[Symbol.iterator]();
        }};}
      }];
      default: throw Error('unknown case '+id);
    }
  }
  let count = 0;
  for (const documentKind of ['live','windowless']) {
    for (const methodKind of ['element','shadow']) {
      for (const [realmName,realm] of [['local',window],['child',other]]) {
        const method = methodKind === 'element' ? realm.Element.prototype.getHTML : realm.ShadowRoot.prototype.getHTML;
        for (const id of cases) {
          const owner = documentKind === 'live' ? document : document.implementation.createHTMLDocument('');
          const host = owner.body.appendChild(owner.createElement('div'));
          host.innerHTML = light;
          const root = host.attachShadow({mode:'open',serializable:true});
          root.innerHTML = '<i>shadow</i>';
          const nestedHost = root.appendChild(owner.createElement('section'));
          const nested = nestedHost.attachShadow({mode:'closed'});
          nested.innerHTML = '<em>nested</em>';
          const log = [], sentinel = {};
          const args = optionsFor(id,root,nested,host,log,sentinel);
          const label = [documentKind,methodKind,realmName,id].join('/');
          let value, error, threw = false;
          try { value = Reflect.apply(method,methodKind === 'element' ? host : root,args); }
          catch (caught) { threw = true; error = caught; }
          try {
            if (typeErrors.has(id)) {
              assert(threw && error instanceof realm.TypeError, label + ': callee realm TypeError');
            } else if (sentinels.has(id)) {
              assert(threw && error === sentinel, label + ': original exception identity');
            } else {
              assert(!threw, label + ': conversion unexpectedly threw');
              let content = fullRoots.has(id) ? complete : plain;
              if (id === 'iterator-mutation') content = content.replace('>shadow<','>changed<').replace('>nested<','>mutated<');
              const expected = methodKind === 'shadow' ? content :
                (fullRoots.has(id) || id === 'serializable' ? outerStart + content + '</template>' + light : light);
              assert(value === expected, label + ': serialized output ' + JSON.stringify({value,expected}));
            }
            assert(JSON.stringify(log) === JSON.stringify(expectedLogs[id] || []), label + ': access order ' + JSON.stringify(log));
            count++;
          } finally { host.remove(); }
        }
      }
    }
  }
  assert(count === cases.length * 8, 'all getHTML contexts must be exercised');
  return 'ok';
}
