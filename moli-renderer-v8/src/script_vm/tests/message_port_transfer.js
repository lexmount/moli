async (owner, callee, label) => {
    const rows = [];
    const assert = (ok, message = 'assertion failed') => { if (!ok) throw new Error(message); };
    const throws = run => {
        try { run(); } catch (error) {
            assert(error instanceof callee.DOMException && error.name === 'DataCloneError', String(error));
            return;
        }
        throw new Error('expected DataCloneError');
    };
    const next = port => new Promise(resolve => { port.onmessage = resolve; });
    for (const mode of ['clone', 'port']) {
        const transport = new owner.MessageChannel();
        const send = (value, transfer) => mode === 'clone'
            ? callee.structuredClone(value, {transfer})
            : Reflect.apply(callee.MessagePort.prototype.postMessage, transport.port1, [value, {transfer}]);
        const check = async (name, run) => {
            try { await run(); rows.push({name: `${label}: ${mode}: ${name}`, passed: true}); }
            catch (error) { rows.push({name: `${label}: ${mode}: ${name}`, passed: false, error: String(error)}); }
        };
        const detach = (port, how) => {
            if (how === 'closed') port.close();
            else callee.structuredClone(port, {transfer: [port]}).close();
        };
        try {
            await check('graph identity, port index and entanglement', async () => {
                const source = new owner.MessageChannel(), unused = new owner.MessageChannel();
                let copy, extras = [];
                try {
                    const value = {first: source.port1, second: source.port1};
                    const buffer = new owner.ArrayBuffer(3);
                    if (mode === 'clone') copy = send(value, [buffer, source.port1, unused.port1]);
                    else {
                        const received = next(transport.port2);
                        send(value, [buffer, source.port1, unused.port1]);
                        const event = await received;
                        copy = event.data; extras = Array.from(event.ports);
                        assert(extras.length === 2 && extras[0] === copy.first, 'MessageEvent.ports identity');
                    }
                    assert(copy.first === copy.second && buffer.byteLength === 0);
                    assert(Object.getPrototypeOf(copy.first) === (mode === 'clone' ? callee : owner).MessagePort.prototype);
                    const received = next(copy.first);
                    source.port2.postMessage('live');
                    assert((await received).data === 'live');
                    throws(() => callee.structuredClone(1, {transfer: [source.port1]}));
                    throws(() => callee.structuredClone(1, {transfer: [unused.port1]}));
                } finally {
                    source.port1.close(); source.port2.close(); unused.port1.close(); unused.port2.close();
                    copy?.first.close(); extras.forEach(port => port.close());
                }
            });
            await check('unreferenced transfer detaches sender', () => {
                const source = new owner.MessageChannel();
                try {
                    send(7, [source.port1]);
                    throws(() => callee.structuredClone(1, {transfer: [source.port1]}));
                } finally { source.port1.close(); source.port2.close(); }
            });
            await check('duplicate transfer rejects before message getters', () => {
                const source = new owner.MessageChannel(); let reads = 0;
                try {
                    throws(() => send({get value() {reads++;}}, [source.port1, source.port1]));
                    assert(reads === 0);
                    callee.structuredClone(source.port1, {transfer: [source.port1]}).close();
                } finally { source.port1.close(); source.port2.close(); }
            });
            await check('port outside transfer list cannot be cloned', () => {
                const source = new owner.MessageChannel();
                try { throws(() => send(source.port1, [])); }
                finally { source.port1.close(); source.port2.close(); }
            });
            for (const how of ['closed', 'transferred']) {
                for (const referenced of [false, true]) {
                    await check(`${how} port defers validation, referenced=${referenced}`, () => {
                        const source = new owner.MessageChannel(), sentinel = new Error('getter wins');
                        detach(source.port1, how);
                        let caught, reads = 0;
                        try {
                            send({port: referenced ? source.port1 : null, get value() {reads++; throw sentinel;}}, [source.port1]);
                        } catch (error) { caught = error; }
                        finally { source.port1.close(); source.port2.close(); }
                        assert(caught === sentinel && reads === 1, 'getter exception identity');
                    });
                }
                await check(`${how} port rejects after successful serialization`, () => {
                    const source = new owner.MessageChannel(); detach(source.port1, how);
                    try { throws(() => send({port: source.port1}, [source.port1])); }
                    finally { source.port2.close(); }
                });
                for (const bufferFirst of [false, true]) {
                    await check(`${how} port preserves transfer order, buffer first=${bufferFirst}`, () => {
                        const source = new owner.MessageChannel(), buffer = new owner.ArrayBuffer(3);
                        detach(source.port1, how);
                        try {
                            throws(() => send({port: source.port1}, bufferFirst ? [buffer, source.port1] : [source.port1, buffer]));
                            assert(buffer.byteLength === (bufferFirst ? 0 : 3));
                        } finally { source.port2.close(); }
                    });
                }
            }
            await check('getter closes referenced port before transfer steps', () => {
                const source = new owner.MessageChannel(), buffer = new owner.ArrayBuffer(3);
                try {
                    throws(() => send({port: source.port1, get value() {source.port1.close();}}, [source.port1, buffer]));
                    assert(buffer.byteLength === 3);
                } finally { source.port1.close(); source.port2.close(); }
            });
            for (const referenced of [false, true]) {
                await check(`getter transfers port before transfer steps, referenced=${referenced}`, () => {
                    const source = new owner.MessageChannel(), buffer = new owner.ArrayBuffer(3);
                    try {
                        throws(() => send({port: referenced ? source.port1 : null, get value() {
                            callee.structuredClone(source.port1, {transfer: [source.port1]}).close();
                        }}, [source.port1, buffer]));
                        assert(buffer.byteLength === 3);
                    } finally { source.port1.close(); source.port2.close(); }
                });
            }
            await check('serialization failure preserves transferable resources', () => {
                const source = new owner.MessageChannel(), buffer = new owner.ArrayBuffer(3);
                try {
                    throws(() => send({port: source.port1, bad() {}}, [source.port1, buffer]));
                    assert(buffer.byteLength === 3);
                    callee.structuredClone(source.port1, {transfer: [source.port1]}).close();
                } finally { source.port1.close(); source.port2.close(); }
            });
            await check('author proxies and forged receivers do not run traps', () => {
                const source = new owner.MessageChannel(); let traps = 0;
                const revoked = owner.Proxy.revocable(source.port1, {}); revoked.revoke();
                try {
                    for (const value of [new owner.Proxy(source.port1, {get() {traps++;}}), revoked.proxy,
                        Object.create(source.port1), Object.create(owner.MessagePort.prototype)]) {
                        throws(() => send(1, [value]));
                    }
                    assert(traps === 0);
                } finally { source.port1.close(); source.port2.close(); }
            });
            await check('native port identity ignores author metadata and constructor changes', async () => {
                const source = new owner.MessageChannel(), prototype = callee.MessagePort.prototype;
                const constructor = callee.MessagePort;
                Object.defineProperty(source.port1, 'endpoint', {get() {throw Error('author endpoint');}});
                Object.freeze(source.port1);
                try {
                    callee.MessagePort = function() {throw Error('author constructor');};
                    const copy = callee.structuredClone(source.port1, {transfer: [source.port1]});
                    assert(Object.getPrototypeOf(copy) === prototype);
                    const received = next(copy);
                    source.port2.postMessage('still usable');
                    assert((await received).data === 'still usable'); copy.close();
                } finally {
                    callee.MessagePort = constructor;
                    source.port1.close(); source.port2.close();
                }
            });
        } finally { transport.port1.close(); transport.port2.close(); }
    }
    return rows;
}
