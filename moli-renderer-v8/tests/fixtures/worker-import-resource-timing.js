async base => {
    const checks = [];
    const test = (name, callback) => {
        try {
            if (!callback()) throw Error('assertion failed');
            checks.push({name, passed: true});
        } catch (error) {
            checks.push({name, passed: false, details: String(error.stack || error)});
        }
    };
    const capture = callback => { try { callback(); } catch (error) { return error; } };
    const resources = () => performance.getEntriesByType('resource');
    const byName = name => performance.getEntriesByName(name, 'resource');
    const tick = () => new Promise(resolve => setTimeout(resolve, 0));
    const observe = promise => new Promise(resolve => {
        const timer = setTimeout(() => resolve(false), 1000);
        promise.then(() => { clearTimeout(timer); resolve(true); });
    });
    const originalEvent = Event;
    const originalPerformance = performance;
    globalThis.__importTimingDuring = [];
    performance.clearResourceTimings();
    performance.setResourceTimingBufferSize(250);
    const deliveries = [];
    const observer = new PerformanceObserver(list => deliveries.push(...list.getEntries()));
    observer.observe({type: 'resource'});
    const paths = [
        ['success', '/timing-import.js?first=1#fragment', null, 200],
        ['repeat', '/timing-import.js?first=1#fragment', null, 200],
        ['redirect', '/timing-redirect.js', null, 200],
        ['HTTP failure', '/timing-missing.js', 'NetworkError', 404],
        ['MIME failure', '/timing-mime.js', 'NetworkError', 200],
        ['execution failure', '/timing-import.js?throws=1', 'Error', 200],
    ];
    try {
        for (let i = 0; i < paths.length; i++) {
            const [label, path, errorName, status] = paths[i];
            const url = base + path;
            const before = performance.now();
            const error = capture(() => importScripts(url));
            const after = performance.now();
            const entries = resources();
            const entry = entries[i];
            test(label + ': one resource per attempt', () => entries.length === i + 1);
            test(label + ': completion preserves script error', () => (error?.name ?? null) === errorName);
            test(label + ': requested URL identity', () => entry.name === url);
            test(label + ': native resource prototype and initiator', () =>
                entry instanceof PerformanceResourceTiming && entry.entryType === 'resource' && entry.initiatorType === 'other');
            test(label + ': start and completion use the worker clock', () =>
                entry.startTime >= before && entry.startTime <= after && entry.duration >= 0 &&
                entry.startTime + entry.duration <= after + 0.01);
            test(label + ': response status survives loader rejection', () => entry.responseStatus === status);
            test(label + ': actual response sizes and protocol', () =>
                entry.encodedBodySize > 0 && entry.decodedBodySize > 0 && entry.transferSize >= entry.encodedBodySize &&
                entry.nextHopProtocol.startsWith('http/1.'));
            test(label + ': header arrival precedes body completion', () =>
                entry.responseStart >= entry.startTime && entry.responseStart <= entry.responseEnd && entry.responseEnd <= after);
            test(label + ': JSON snapshot preserves name and status', () =>
                entry.toJSON().name === url && entry.toJSON().responseStatus === status);
        }
        test('entry is visible to the first imported script during execution', () =>
            __importTimingDuring[0].length === 1 && __importTimingDuring[0][0] === base + paths[0][1]);
        test('repeated imports retain distinct native entries', () => {
            const entries = byName(base + paths[0][1]);
            return entries.length === 2 && entries[0] !== entries[1];
        });
        test('redirect uses the requested URL rather than response URL', () =>
            byName(base + '/timing-redirect.js').length === 1 && byName(base + '/timing-import.js?redirected=1').length === 0);
        await tick();
        test('observer receives the same native entries including rejected imports', () => {
            const entries = resources();
            return deliveries.length === entries.length && entries.every(entry => deliveries.includes(entry));
        });
        const cross = base.replace('127.0.0.1', 'localhost');
        for (const tao of [false, true]) {
            const url = cross + '/timing-import.js?cross=1' + (tao ? '&tao=1' : '');
            const error = capture(() => importScripts(url));
            const entry = byName(url)[0];
            test('cross-origin ' + tao + ': script executes and has a resource entry', () => !error && entry !== undefined);
            test('cross-origin ' + tao + ': opaque response status stays hidden', () => entry.responseStatus === 0);
            test('cross-origin ' + tao + ': sizes follow TAO', () => tao ?
                entry.encodedBodySize > 0 && entry.decodedBodySize > 0 && entry.transferSize > 0 :
                entry.encodedBodySize === 0 && entry.decodedBodySize === 0 && entry.transferSize === 0);
            test('cross-origin ' + tao + ': connection timing follows TAO', () => tao ?
                entry.responseStart > 0 && entry.nextHopProtocol !== '' : entry.responseStart === 0 && entry.nextHopProtocol === '');
        }
        performance.clearResourceTimings();
        performance.setResourceTimingBufferSize(1);
        const overflowURL = base + '/timing-import.js?overflow=1';
        importScripts(base + '/timing-import.js?primary=1');
        importScripts(overflowURL);
        const immediate = observer.takeRecords();
        test('observer queue includes resources excluded from primary buffer', () => immediate.some(e => e.name === overflowURL));
        test('primary buffer enforces its configured size synchronously', () => resources().length === 1 && byName(overflowURL).length === 0);
        let fullEvent;
        const full = new Promise(resolve => {
            performance.onresourcetimingbufferfull = event => {
                fullEvent = event;
                performance.setResourceTimingBufferSize(4);
                resolve();
            };
        });
        globalThis.Event = function() { throw Error('author Event constructor'); };
        await observe(full);
        globalThis.Event = originalEvent;
        await tick();
        test('buffer-full is a native trusted event', () =>
            fullEvent instanceof originalEvent && fullEvent.isTrusted && fullEvent.target === performance);
        test('growing the buffer during the event retains overflow', () => resources().length === 2 && byName(overflowURL).length === 1);
        performance.clearResourceTimings();
        performance.setResourceTimingBufferSize(0);
        importScripts(base + '/timing-import.js?clear-pending=1');
        performance.clearResourceTimings();
        const afterClear = new Promise(resolve => {
            performance.onresourcetimingbufferfull = () => {
                performance.setResourceTimingBufferSize(1);
                resolve();
            };
        });
        await observe(afterClear);
        await tick();
        test('clearing primary entries preserves pending secondary-buffer work', () =>
            resources().length === 1 && resources()[0].name === base + '/timing-import.js?clear-pending=1');
        const mark = performance.mark('__workerResourceControl', {startTime: 1});
        performance.clearResourceTimings();
        test('clearResourceTimings preserves user timing', () => performance.getEntriesByName(mark.name, 'mark')[0] === mark);
        performance.clearMarks(mark.name);
        test('buffer size conversion still uses Web IDL unsigned long', () =>
            capture(() => performance.setResourceTimingBufferSize(Symbol())) instanceof TypeError);
        const descriptor = Object.getOwnPropertyDescriptor(globalThis, 'performance');
        try {
            Object.defineProperty(globalThis, 'performance', {value: {}, configurable: true});
            importScripts(base + '/timing-noop.js');
        } finally {
            if (descriptor) Object.defineProperty(globalThis, 'performance', descriptor);
            else delete globalThis.performance;
        }
        test('publishing uses the native Performance despite author shadowing', () =>
            originalPerformance.getEntriesByName(base + '/timing-noop.js', 'resource').length === 1);
    } finally {
        globalThis.Event = originalEvent;
        observer.disconnect();
        originalPerformance.onresourcetimingbufferfull = null;
        originalPerformance.clearResourceTimings();
        originalPerformance.setResourceTimingBufferSize(250);
        delete globalThis.__importTimingDuring;
    }
    return checks;
}
