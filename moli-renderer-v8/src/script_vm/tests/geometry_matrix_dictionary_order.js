(() => {
  const checks = [];
  const check = (name, fn) => {
    try { checks.push({name, passed: fn() === true}); }
    catch (error) { checks.push({name, passed: false, error: String(error)}); }
  };
  const realms = [window, document.querySelector('iframe').contentWindow];
  const base = ['a', 'b', 'c', 'd', 'e', 'f', 'm11', 'm12', 'm21', 'm22', 'm41', 'm42'];
  const own = ['is2D', 'm13', 'm14', 'm23', 'm24', 'm31', 'm32', 'm33', 'm34', 'm43', 'm44'];
  const names = [...base, ...own];
  for (const [ownerIndex, owner] of realms.entries()) {
    for (const [calleeIndex, callee] of realms.entries()) {
      const calls = [
        ['DOMMatrix.fromMatrix', value => callee.DOMMatrix.fromMatrix(value)],
        ['DOMMatrixReadOnly.fromMatrix', value => callee.DOMMatrixReadOnly.fromMatrix(value)],
        ['DOMPoint.matrixTransform', value => new callee.DOMPoint(1, 2, 3, 1).matrixTransform(value)],
        ['DOMMatrix.multiply', value => new callee.DOMMatrix().multiply(value)],
        ['DOMMatrix.multiplySelf', value => new callee.DOMMatrix().multiplySelf(value)],
        ['DOMMatrix.preMultiplySelf', value => new callee.DOMMatrix().preMultiplySelf(value)],
      ];
      for (const [name, call] of calls) {
        const prefix = `matrix-order/${ownerIndex}/${calleeIndex}/${name}`;
        check(prefix + '/getter-conversion-order', () => {
          const order = [], value = {};
          for (const key of names) Object.defineProperty(value, key, {get() {
            order.push(key);
            return key === 'is2D' ? false : {valueOf() {order.push(key + ':number'); return 1;}};
          }});
          call(value);
          const expected = names.flatMap(key => key === 'is2D' ? [key] : [key, key + ':number']);
          return order.join() === expected.join();
        });
        for (const proxy of [false, true]) check(prefix + '/inherited-or-proxy/' + proxy, () => {
          const order = [], target = {};
          for (const key of names) Object.defineProperty(target, key, {get() {order.push(key); return key === 'is2D' ? false : 1;}});
          call(proxy ? new owner.Proxy(target, {}) : owner.Object.create(target));
          return order.join() === names.join();
        });
        for (const key of names) for (const conversion of [false, true]) {
          if (conversion && key === 'is2D') continue;
          check(prefix + '/exception/' + key + '/' + conversion, () => {
            const sentinel = {}, order = [], value = {};
            for (const member of names) Object.defineProperty(value, member, {get() {
              order.push(member);
              if (member === key) {
                if (!conversion) throw sentinel;
                return {valueOf() {order.push(member + ':number'); throw sentinel;}};
              }
              return member === 'is2D' ? false : 1;
            }});
            try { call(value); } catch (error) {
              const expected = names.slice(0, names.indexOf(key) + 1);
              if (conversion) expected.push(key + ':number');
              return error === sentinel && order.join() === expected.join();
            }
            return false;
          });
        }
        for (const [index, value] of [undefined, null, {}, [], () => {}].entries()) {
          check(prefix + '/empty/' + index, () => {call(value); return true;});
        }
        for (const [index, value] of [0, true, 'matrix', Symbol(), 1n].entries()) {
          check(prefix + '/invalid/' + index, () => {
            try {call(value);} catch (error) {return Object.getPrototypeOf(error) === callee.TypeError.prototype;}
            return false;
          });
        }
        check(prefix + '/validation-after-all-members', () => {
          const order = [], sentinel = {};
          const value = new owner.Proxy({a: 1, m11: 2}, {get(target, key) {
            order.push(key);
            if (key === 'm44') throw sentinel;
            return target[key];
          }});
          try {call(value);} catch (error) {return error === sentinel && order.join() === names.join();}
          return false;
        });
        check(prefix + '/boolean-does-not-coerce', () => {
          let conversions = 0;
          call({is2D: {valueOf() {conversions++; throw Error('boolean coercion');}}});
          return conversions === 0;
        });
      }
    }
  }
  globalThis.__matrixDictionaryOrderResults = {complete: true, total: checks.length, passed: checks.filter(row => row.passed).length, checks};
  return true;
})()
