(() => {
  'use strict';
  const checks = [];
  function check(name, predicate) {
    try {
      checks.push({name, passed: !!predicate()});
    } catch (error) {
      checks.push({name, passed: false, error: error.name + ': ' + error.message});
    }
  }
  function thrown(fn) {
    try { fn(); } catch (error) { return error; }
  }
  const fields = ['x', 'y', 'width', 'height'];
  const sides = ['top', 'right', 'bottom', 'left'];
  const values = [12, 34, -30, -40];
  const expected = {x:12,y:34,width:-30,height:-40,top:-6,right:12,bottom:34,left:-18};
  const realms = [['main', globalThis], ['child', document.querySelector('iframe').contentWindow]];
  const descriptor = (realm, kind, name) => Object.getOwnPropertyDescriptor(realm[kind].prototype, name);
  function invalids(realm, real, counters) {
    const revoked = Proxy.revocable(real, {}); revoked.revoke();
    return [null, undefined, 1, 'rect', {}, Object.create(real),
      Object.create(Object.getPrototypeOf(real)),
      new Proxy(real, {get(){ counters.traps++; throw Error('proxy trap'); }}), revoked.proxy,
      new realm.DOMPoint(), realm.document.createElement('div')];
  }
  for (const [label, realm] of realms) {
    for (const kind of ['DOMRect', 'DOMRectReadOnly']) {
      const real = new realm[kind](...values);
      for (const [callerLabel, caller] of realms) {
        const counters = {traps:0,conversions:0};
        for (const name of fields.concat(sides)) {
          const proto = fields.includes(name) ? kind : 'DOMRectReadOnly';
          const get = descriptor(caller, proto, name).get;
          check(`${label}/${kind}/${callerLabel}/${name}/genuine`, () => get.call(real) === expected[name]);
          invalids(realm, real, counters).forEach((receiver, index) => {
            check(`${label}/${kind}/${callerLabel}/${name}/invalid${index}`, () => thrown(() => get.call(receiver)) instanceof caller.TypeError);
          });
        }
        const json = caller.DOMRectReadOnly.prototype.toJSON;
        check(`${label}/${kind}/${callerLabel}/json values`, () => {
          const result = json.call(real);
          return Object.keys(expected).every(name => Object.is(result[name], expected[name]));
        });
        check(`${label}/${kind}/${callerLabel}/json realm`, () => Object.getPrototypeOf(json.call(real)) === caller.Object.prototype);
        invalids(realm, real, counters).forEach((receiver, index) => {
          check(`${label}/${kind}/${callerLabel}/json invalid${index}`, () => thrown(() => json.call(receiver)) instanceof caller.TypeError);
        });
        for (const name of fields) {
          const mutable = descriptor(caller, 'DOMRect', name);
          const rhs = {valueOf(){ counters.conversions++; return 91; }};
          for (const [index, receiver] of invalids(realm, real, counters).entries()) {
            check(`${label}/${kind}/${callerLabel}/${name}/setter invalid${index}`, () => thrown(() => mutable.set.call(receiver, rhs)) instanceof caller.TypeError);
          }
          const readonly = new realm.DOMRectReadOnly(...values);
          check(`${label}/${kind}/${callerLabel}/${name}/mutable getter rejects readonly`, () => thrown(() => mutable.get.call(readonly)) instanceof caller.TypeError);
          check(`${label}/${kind}/${callerLabel}/${name}/mutable setter rejects readonly`, () => thrown(() => mutable.set.call(readonly, rhs)) instanceof caller.TypeError);
          check(`${label}/${kind}/${callerLabel}/${name}/readonly unchanged`, () => readonly[name] === expected[name]);
        }
        check(`${label}/${kind}/${callerLabel}/invalid no conversion`, () => counters.conversions === 0);
        check(`${label}/${kind}/${callerLabel}/invalid no proxy traps`, () => counters.traps === 0);
      }
      check(`${label}/${kind}/own slots hidden`, () => fields.concat(sides).every(name => !Object.hasOwn(real, name)));
      check(`${label}/${kind}/clone snapshot`, () => {
        const clone = structuredClone(real);
        return Object.getPrototypeOf(clone) === globalThis[kind].prototype && Object.keys(expected).every(name => Object.is(clone[name], expected[name]));
      });
      for (const [callerLabel, caller] of realms) {
        const get = descriptor(caller, 'DOMRectReadOnly', 'x').get;
        check(`${label}/${kind}/${callerLabel}/prototype change retains brand`, () => {
          const rect = new realm[kind](7); Object.setPrototypeOf(rect, null);
          return get.call(rect) === 7;
        });
      }
    }
    for (const name of fields) {
      const d = descriptor(realm, 'DOMRect', name);
      check(`${label}/${name}/setter converts once`, () => {
        const rect = new realm.DOMRect(); let conversions = 0;
        const result = d.set.call(rect, {valueOf(){ conversions++; return .1; }});
        return result === undefined && conversions === 1 && rect[name] === .1;
      });
      for (const [index, value] of [NaN,Infinity,-Infinity,-0,1e300,Number.MIN_VALUE].entries()) {
        check(`${label}/${name}/unrestricted${index}`, () => {
          const rect = new realm.DOMRect(); d.set.call(rect, value);
          return Object.is(rect[name], value);
        });
      }
      check(`${label}/${name}/throwing conversion unchanged`, () => {
        const rect = new realm.DOMRect(...values), error = {};
        return thrown(() => d.set.call(rect, {valueOf(){throw error;}})) === error && rect[name] === expected[name];
      });
      for (const [index, value] of [Symbol(), 1n].entries()) {
        check(`${label}/${name}/conversion TypeError${index}`, () => thrown(() => d.set.call(new realm.DOMRect(), value)) instanceof realm.TypeError);
      }
    }
    for (const [constructorName, kind] of [['DOMRect','DOMRect'],['DOMRectReadOnly','DOMRectReadOnly']]) {
      const source = new realm[constructorName](...values);
      check(`${label}/${kind}/fromRect prototype`, () => Object.getPrototypeOf(realm[kind].fromRect(source)) === realm[kind].prototype);
      check(`${label}/${kind}/fromRect snapshot`, () => {
        const clone = realm[kind].fromRect(source);
        return clone !== source && Object.keys(expected).every(name => Object.is(clone[name], expected[name]));
      });
    }
  }
  const ns = 'http://www.w3.org/2000/svg';
  for (const [label, realm] of realms) {
    const docs = [['live', realm.document], ['html',realm.document.implementation.createHTMLDocument('')],
      ['xml',new realm.DOMParser().parseFromString('<root/>','application/xml')]];
    for (const [docLabel, doc] of docs) {
      for (const tag of ['svg','symbol','marker','pattern','view']) {
        const prefix = `${label}/${docLabel}/${tag}`;
        const owner = doc.createElementNS(ns,tag);
        const animated = owner.viewBox, base = animated.baseVal, anim = animated.animVal;
        check(`${prefix}/animated prototype`, () => Object.getPrototypeOf(animated) === realm.SVGAnimatedRect.prototype);
        check(`${prefix}/initial no attribute`, () => !owner.hasAttribute('viewBox'));
        check(`${prefix}/initial values`, () => fields.every(name => base[name] === 0 && anim[name] === 0));
        check(`${prefix}/SameObject distinct modes`, () => owner.viewBox === animated && animated.baseVal === base && animated.animVal === anim && base !== anim);
        for (const [callerLabel, caller] of realms) {
          const counters = {traps:0,conversions:0};
          for (const name of ['baseVal','animVal']) {
            const get = descriptor(caller,'SVGAnimatedRect',name).get;
            check(`${prefix}/${callerLabel}/${name}/genuine`, () => get.call(animated) === (name === 'baseVal' ? base : anim));
            invalids(realm,animated,counters).forEach((receiver,index) => {
              check(`${prefix}/${callerLabel}/${name}/invalid${index}`, () => thrown(() => get.call(receiver)) instanceof caller.TypeError);
            });
          }
          for (const name of fields) {
            const d = descriptor(caller,'SVGRect',name);
            const rhs = {valueOf(){counters.conversions++; return 1;}};
            invalids(realm,base,counters).forEach((receiver,index) => {
              check(`${prefix}/${callerLabel}/${name}/getter invalid${index}`, () => thrown(() => d.get.call(receiver)) instanceof caller.TypeError);
              check(`${prefix}/${callerLabel}/${name}/setter invalid${index}`, () => thrown(() => d.set.call(receiver,rhs)) instanceof caller.TypeError);
            });
            check(`${prefix}/${callerLabel}/${name}/readonly brand precedes conversion`, () => {
              let conversions = 0;
              const error = thrown(() => d.set.call(anim,{valueOf(){conversions++;return 1;}}));
              return conversions === 0 && error instanceof caller.TypeError;
            });
            check(`${prefix}/${callerLabel}/${name}/readonly throwing conversion`, () => {
              const error = {};
              return thrown(() => d.set.call(anim,{valueOf(){throw error;}})) instanceof caller.TypeError;
            });
            for (const [index,value] of [NaN,Infinity,-Infinity,1e40,undefined,Symbol(),1n].entries()) {
              check(`${prefix}/${callerLabel}/${name}/readonly invalid numeric${index}`, () => thrown(() => d.set.call(anim,value)) instanceof caller.TypeError);
            }
          }
          check(`${prefix}/${callerLabel}/invalid no conversion`, () => counters.conversions === 0);
          check(`${prefix}/${callerLabel}/invalid no proxy traps`, () => counters.traps === 0);
        }
        owner.setAttribute('viewBox','10 20 30 40');
        for (const [index,name] of fields.entries()) {
          check(`${prefix}/${name}/retained synchronized`, () => base[name] === [10,20,30,40][index] && anim[name] === [10,20,30,40][index]);
        }
        for (const [index,name] of fields.entries()) {
          check(`${prefix}/${name}/one field reentrant mutation`, () => {
            let conversions = 0;
            base[name] = {valueOf(){conversions++;owner.setAttribute('viewBox','50 60 70 80');return 91+index;}};
            const expected = [50,60,70,80];expected[index]=91+index;
            return conversions === 1 && owner.getAttribute('viewBox') === expected.join(' ') && fields.every((key,i) => base[key] === expected[i] && anim[key] === expected[i]);
          });
        }
        owner.removeAttribute('viewBox');
        check(`${prefix}/retained removed resets`, () => fields.every(name => base[name] === 0 && anim[name] === 0));
        owner.setAttribute('viewBox','0 0 -1 2');
        check(`${prefix}/retained invalid resets`, () => fields.every(name => base[name] === 0 && anim[name] === 0));
        owner.setAttribute('viewBox','1 2 3 4');
        check(`${prefix}/prototype change retains state`, () => {
          Object.setPrototypeOf(base,null);
          return descriptor(realm,'SVGRect','x').get.call(base) === 1;
        });
      }
      const root = doc.createElementNS(ns,'svg');
      for (const [callerLabel,caller] of realms) {
        const factory = caller.SVGSVGElement.prototype.createSVGRect;
        check(`${label}/${docLabel}/${callerLabel}/factory realm`, () => Object.getPrototypeOf(factory.call(root)) === caller.SVGRect.prototype);
        check(`${label}/${docLabel}/${callerLabel}/factory detached independent`, () => {
          const a=factory.call(root),b=factory.call(root);a.x=5;
          return a!==b && a.x===5 && b.x===0 && !root.hasAttribute('viewBox');
        });
        const counters = {traps:0,conversions:0};
        invalids(realm,root,counters).forEach((receiver,index) => {
          check(`${label}/${docLabel}/${callerLabel}/factory invalid${index}`, () => thrown(() => factory.call(receiver)) instanceof caller.TypeError);
        });
        check(`${label}/${docLabel}/${callerLabel}/factory no proxy traps`, () => counters.traps===0);
      }
    }
  }
  globalThis.__uiEventResults = {complete:true,total:checks.length,passed:checks.filter(row=>row.passed).length,checks};
  return checks.every(row=>row.passed);
})()
