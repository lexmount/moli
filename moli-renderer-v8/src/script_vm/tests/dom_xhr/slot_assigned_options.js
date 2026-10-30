globalThis.slotAssignedOptionsProbe = variant => {
  const check = (condition, label) => {
    if (!condition) throw new Error(label);
  };
  const other = document.querySelector('iframe').contentWindow;
  check(other.Object !== Object, 'the callee realm must be distinct');
  check(other.TypeError !== TypeError, 'the TypeError intrinsics must be distinct');
  const documents = [document, document.implementation.createHTMLDocument('windowless')];
  for (const [ownerIndex, owner] of documents.entries()) {
    for (const [realmIndex, realm] of [window, other].entries()) {
      for (const name of ['assignedNodes', 'assignedElements']) {
        const label = `${variant}/${ownerIndex}/${realmIndex}/${name}`;
        const host = owner.body.appendChild(owner.createElement('div'));
        const root = host.attachShadow({mode: 'open'});
        root.innerHTML = '<slot id="outer"><slot><b id="leaf"></b></slot></slot>';
        const slot = root.querySelector('#outer');
        const leaf = root.querySelector('#leaf');
        const method = realm.HTMLSlotElement.prototype[name];
        const call = value => method.call(slot, value);
        const expectFlat = (value, flatten, message) => {
          check(value.length === (flatten ? 1 : 0), `${label}: ${message} length`);
          if (flatten) check(value[0] === leaf, `${label}: ${message} node identity`);
        };
        const expectTypeError = (operation, message) => {
          let error;
          try { operation(); } catch (caught) { error = caught; }
          check(error instanceof realm.TypeError, `${label}: ${message} callee TypeError`);
          if (realmIndex) check(!(error instanceof TypeError), `${label}: ${message} caller TypeError`);
        };
        if (variant === 'dictionary') {
          expectFlat(method.call(slot), false, 'omitted dictionary');
          for (const value of [undefined, null, {}, [], function () {}]) {
            expectFlat(call(value), false, 'empty dictionary');
          }
          for (const value of [true, false, 0, 1, NaN, '', 'options', 1n, Symbol('options')]) {
            expectTypeError(() => call(value), 'primitive dictionary');
          }
          for (const [value, flatten] of [
            [false, false], [0, false], ['', false], [null, false], [undefined, false],
            [NaN, false], [0n, false], ['false', true], [1, true], [Symbol('truthy'), true],
            [{}, true], [[], true],
          ]) {
            expectFlat(call({flatten: value}), flatten, 'boolean dictionary member');
          }
          let reads = 0;
          const inherited = Object.create({get flatten() { ++reads; return true; }});
          expectFlat(call(inherited), true, 'inherited member');
          check(reads === 1, `${label}: inherited getter read once`);
          const callable = Object.assign(function () {}, {flatten: true});
          expectFlat(call(callable), true, 'callable dictionary');
          const nullPrototype = Object.assign(Object.create(null), {flatten: true});
          expectFlat(call(nullPrototype), true, 'null prototype dictionary');
          let coercions = 0;
          expectFlat(call({flatten: {[Symbol.toPrimitive]() { ++coercions; throw 1; }}}), true, 'object boolean');
          check(coercions === 0, `${label}: ToBoolean must not invoke user conversion`);
          const keys = [];
          const options = new Proxy({}, {
            get(_target, key) { keys.push(key); return undefined; },
            has() { throw new Error('dictionary must use Get'); },
            ownKeys() { throw new Error('dictionary must not enumerate'); },
          });
          expectFlat(call(options), false, 'proxy dictionary');
          check(keys.length === 1 && keys[0] === 'flatten', `${label}: only read flatten`);
          expectFlat(call({get flattened() { throw new Error('not a dictionary member'); }}), false, 'legacy spelling');
        } else if (variant === 'exceptions') {
          const marker = {};
          const reads = [];
          let caught;
          try {
            call({
              get flatten() { reads.push('flatten'); throw marker; },
              get flattened() { reads.push('flattened'); throw new Error('unexpected getter'); },
            });
          } catch (error) { caught = error; }
          check(caught === marker, `${label}: preserve the thrown object`);
          check(reads.join(',') === 'flatten', `${label}: stop at the throwing member`);
          const pair = Proxy.revocable({}, {});
          pair.revoke();
          expectTypeError(() => call(pair.proxy), 'revoked dictionary');
          let conversions = 0;
          let traps = 0;
          const options = {get flatten() { ++conversions; return true; }};
          const invalidProxy = new Proxy(slot, {get() { ++traps; throw marker; }});
          const revokedReceiver = Proxy.revocable(slot, {});
          revokedReceiver.revoke();
          for (const receiver of [
            {}, owner.createElement('div'), Object.create(slot),
            Object.create(realm.HTMLSlotElement.prototype), invalidProxy, revokedReceiver.proxy,
          ]) {
            expectTypeError(() => method.call(receiver, options), 'invalid receiver');
          }
          check(conversions === 0, `${label}: receiver validation precedes dictionary conversion`);
          check(traps === 0, `${label}: receiver validation must not invoke proxy traps`);
          let firstReads = 0;
          let secondReads = 0;
          expectFlat(method.call(slot, {get flatten() { ++firstReads; return false; }}, {
            get flatten() { ++secondReads; throw marker; },
          }), false, 'extra arguments');
          check(firstReads === 1 && secondReads === 0, `${label}: convert only the first dictionary`);
        } else if (variant === 'mutation') {
          slot.name = 'before';
          const before = owner.createElement('span');
          before.slot = 'before';
          host.appendChild(before);
          const after = owner.createElement('i');
          after.slot = 'after';
          let reads = 0;
          const result = call({get flatten() {
            ++reads;
            slot.name = 'after';
            host.appendChild(after);
            const nested = call({flatten: false});
            check(nested.length === 1 && nested[0] === after, `${label}: reentrant current assignment`);
            return false;
          }});
          check(reads === 1, `${label}: mutation getter read once`);
          check(result.length === 1 && result[0] === after, `${label}: query assignment after conversion`);
        } else {
          throw new Error(`unknown probe ${variant}`);
        }
        host.remove();
      }
    }
  }
  return 'ok';
};
