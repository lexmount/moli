(() => {
  'use strict';
  const checks = [], ns = 'http://www.w3.org/2000/svg';
  const check = (name, run) => {
    try { checks.push({name, passed: run() === true}); }
    catch (error) { checks.push({name, passed: false, error: String(error)}); }
  };
  const thrown = run => { try { run(); } catch (error) { return error; } };
  const fields = ['x', 'y', 'width', 'height'];
  const frame = document.createElement('iframe'); document.body.appendChild(frame);
  const popup = window.open('about:blank');
  try {
    for (const [label, w] of [['main', window], ['iframe', frame.contentWindow], ['popup', popup]]) {
      if (!w) { check(`${label}/realm available`, () => false); continue; }
      check(`${label}/Window alias`, () => w.SVGRect === w.DOMRect);
      check(`${label}/alias descriptor`, () => {
        const d = Object.getOwnPropertyDescriptor(w, 'SVGRect');
        return d.value === w.DOMRect && d.writable && d.configurable && !d.enumerable;
      });
      check(`${label}/alias constructor`, () => new w.SVGRect(1,2,3,4).height === 4);
      check(`${label}/alias fromRect`, () => w.SVGRect.fromRect({width: .1}).width === .1);
      const docs = [
        ['live', w.document], ['html', w.document.implementation.createHTMLDocument('')],
        ['xml', w.document.implementation.createDocument(null, 'root')],
        ['parser-html', new w.DOMParser().parseFromString('<p>fixture</p>', 'text/html')],
        ['parser-xml', new w.DOMParser().parseFromString('<root/>', 'application/xml')],
      ];
      for (const [provider, doc] of docs) {
        for (const tag of ['svg','symbol','marker','pattern','view']) {
          const prefix = `${label}/${provider}/${tag}`;
          const owner = doc.createElementNS(ns, tag);
          const animated = owner.viewBox, base = animated.baseVal, anim = animated.animVal;
          check(`${prefix}/base type`, () => base instanceof w.DOMRect && Object.getPrototypeOf(base) === w.DOMRect.prototype);
          check(`${prefix}/anim type`, () => anim instanceof w.DOMRectReadOnly && !(anim instanceof w.DOMRect) && Object.getPrototypeOf(anim) === w.DOMRectReadOnly.prototype);
          check(`${prefix}/SameObject`, () => owner.viewBox === animated && animated.baseVal === base && animated.animVal === anim && base !== anim);
          check(`${prefix}/initial no attribute`, () => !owner.hasAttribute('viewBox') && base.x === 0 && anim.width === 0);
          owner.setAttribute('viewBox', '1.2 2.4 3.6 4.8');
          for (const [kind, rect] of [['DOMRect',base], ['DOMRectReadOnly',anim]]) {
            check(`${prefix}/${kind}/double precision`, () => fields.every((name,i) => rect[name] === [1.2,2.4,3.6,4.8][i]));
            check(`${prefix}/${kind}/private storage`, () => fields.every(name => !Object.hasOwn(rect,name)));
            check(`${prefix}/${kind}/JSON reads native slots`, () => {
              owner.setAttribute('viewBox','10 20 30 40');
              Object.defineProperty(rect,'x',{get(){throw Error('author getter');},configurable:true});
              try {
                const json = w.DOMRectReadOnly.prototype.toJSON.call(rect);
                return Object.getPrototypeOf(json) === w.Object.prototype && json.x === 10 && json.right === 40 && json.bottom === 60;
              } finally { delete rect.x; }
            });
            check(`${prefix}/${kind}/clone synchronizes native slots`, () => {
              owner.setAttribute('viewBox','50 60 70 80');
              Object.defineProperty(rect,'x',{get(){throw Error('author getter');},configurable:true});
              try {
                const clone = structuredClone(rect);
                owner.setAttribute('viewBox','1 2 3 4');
                return Object.getPrototypeOf(clone) === globalThis[kind].prototype && clone.x === 50 && clone.right === 120 && clone.bottom === 140 && rect.y === 2;
              } finally { delete rect.x; }
            });
            check(`${prefix}/${kind}/clone is detached`, () => {
              const clone = structuredClone(rect), previous = owner.getAttribute('viewBox');
              if (kind === 'DOMRect') clone.width = .1;
              return owner.getAttribute('viewBox') === previous && (kind !== 'DOMRect' || clone.width === .1);
            });
            check(`${prefix}/${kind}/cross realm JSON`, () => {
              owner.setAttribute('viewBox','10 20 30 40');
              const json = DOMRectReadOnly.prototype.toJSON.call(rect);
              return Object.getPrototypeOf(json) === Object.prototype && fields.every((name,i) => json[name] === [10,20,30,40][i]) && json.right === 40 && json.bottom === 60;
            });
            check(`${prefix}/${kind}/borrowed coordinate getters`, () => {
              owner.setAttribute('viewBox','11 22 33 44');
              return fields.every((name,i) => Object.getOwnPropertyDescriptor(DOMRectReadOnly.prototype,name).get.call(rect) === [11,22,33,44][i]);
            });
            check(`${prefix}/${kind}/borrowed side getters`, () => {
              owner.setAttribute('viewBox','-5 -7 3 4');
              return ['left','top','right','bottom'].every((name,i) => Object.getOwnPropertyDescriptor(DOMRectReadOnly.prototype,name).get.call(rect) === [-5,-7,-2,-3][i]);
            });
            check(`${prefix}/${kind}/fromRect snapshot`, () => {
              const copy = w.DOMRect.fromRect(rect);
              const x = copy.x; owner.setAttribute('viewBox','5 6 7 8');
              return copy !== rect && copy.x === x && rect.x === 5;
            });
            owner.setAttribute('viewBox','1.2 2.4 3.6 4.8');
          }
          for (const name of fields) {
            const setter = Object.getOwnPropertyDescriptor(w.DOMRect.prototype,name).set;
            check(`${prefix}/${name}/readonly rejects before conversion`, () => {
              let conversions = 0;
              const error = thrown(() => setter.call(anim,{valueOf(){conversions++;throw Error('conversion');}}));
              return error instanceof w.TypeError && conversions === 0;
            });
            check(`${prefix}/${name}/readonly strict assignment`, () => thrown(() => { anim[name] = 3; }) instanceof TypeError);
            check(`${prefix}/${name}/double reentrant mutation`, () => {
              let conversions = 0;
              setter.call(base,{valueOf(){conversions++;owner.setAttribute('viewBox','10 20 30 40');return .1;}});
              const expected = [10,20,30,40]; expected[fields.indexOf(name)] = .1;
              return conversions === 1 && fields.every((key,i) => base[key] === expected[i] && anim[key] === expected[i]) && owner.getAttribute('viewBox') === expected.join(' ');
            });
            check(`${prefix}/${name}/borrowed setter reentrant mutation`, () => {
              let conversions = 0;
              const borrowed = Object.getOwnPropertyDescriptor(DOMRect.prototype,name).set;
              borrowed.call(base,{valueOf(){conversions++;owner.setAttribute('viewBox','50 60 70 80');return .2;}});
              const expected = [50,60,70,80]; expected[fields.indexOf(name)] = .2;
              return conversions === 1 && fields.every((key,i) => base[key] === expected[i] && anim[key] === expected[i]) && owner.getAttribute('viewBox') === expected.join(' ');
            });
            check(`${prefix}/${name}/throwing conversion preserved`, () => {
              const sentinel = {}, previous = owner.getAttribute('viewBox');
              return thrown(() => setter.call(base,{valueOf(){throw sentinel;}})) === sentinel && owner.getAttribute('viewBox') === previous;
            });
            check(`${prefix}/${name}/invalid conversion unchanged`, () => {
              const previous = owner.getAttribute('viewBox');
              return thrown(() => setter.call(base,Symbol())) instanceof w.TypeError && owner.getAttribute('viewBox') === previous;
            });
            check(`${prefix}/${name}/author Proxy has no brand`, () => {
              let traps = 0, conversions = 0;
              const proxy = new w.Proxy(base,{get(){traps++;throw Error('trap');}});
              const error = thrown(() => setter.call(proxy,{valueOf(){conversions++;return .1;}}));
              return error instanceof w.TypeError && traps === 0 && conversions === 0;
            });
          }
          check(`${prefix}/large finite viewBox`, () => {
            owner.setAttribute('viewBox','0 0 1e40 1e-300');
            return base.width === 1e40 && anim.height === 1e-300;
          });
          check(`${prefix}/removed resets sides and JSON`, () => {
            owner.removeAttribute('viewBox');
            const json = anim.toJSON();
            return base.right === 0 && json.x === 0 && json.bottom === 0 && !owner.hasAttribute('viewBox');
          });
          check(`${prefix}/invalid resets retained values`, () => {
            owner.setAttribute('viewBox','0 0 -1 2');
            return fields.every(name => base[name] === 0 && anim[name] === 0);
          });
        }
        const root = doc.createElementNS(ns,'svg');
        check(`${label}/${provider}/factory alias and detached clone`, () => {
          const rect = root.createSVGRect(); rect.width = .1;
          const clone = structuredClone(rect);
          return Object.getPrototypeOf(rect) === w.DOMRect.prototype && clone.width === .1 && !root.hasAttribute('viewBox');
        });
        for (const [index,value] of [NaN,Infinity,-Infinity,-0,1e300,Number.MIN_VALUE,undefined].entries()) {
          check(`${label}/${provider}/factory unrestricted ${index}`, () => {
            const rect = root.createSVGRect(); rect.x = value;
            return Object.is(rect.x,Number(value)) && Object.is(rect.toJSON().x,Number(value));
          });
        }
        check(`${label}/${provider}/factory ignores public constructors`, () => {
          const original = w.DOMRect, alias = w.SVGRect, proto = original.prototype;
          try {
            w.DOMRect = w.SVGRect = function(){throw Error('public constructor');};
            return Object.getPrototypeOf(root.createSVGRect()) === proto && Object.getPrototypeOf(doc.createElementNS(ns,'svg').viewBox.baseVal) === proto;
          } finally { w.DOMRect = original; w.SVGRect = alias; }
        });
      }
    }
  } finally { frame.remove(); if (popup) popup.close(); }
  globalThis.__uiEventResults = {complete:true,total:checks.length,passed:checks.filter(row=>row.passed).length,checks};
  return checks.every(row=>row.passed);
})()
