async (owner = globalThis, callee = owner) => {
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
    const performance = owner.performance;
    const method = name => (...args) => Reflect.apply(callee.Performance.prototype[name], performance, args);
    const mark = method('mark');
    const measure = method('measure');
    const byName = method('getEntriesByName');
    const clearMarks = method('clearMarks');
    const clearMeasures = method('clearMeasures');
    const json = entry => Reflect.apply(callee.PerformanceEntry.prototype.toJSON, entry, []);
    const prefix = '__performanceName:';
    const suffixes = ['\ud800', '\ud801', '\udc00', '\udc01', '\ufffd',
        '\ud83d\ude00', '\ud83d', '\ude00', '\0', 'x\0y', '', 'ascii'];
    const names = suffixes.map(value => prefix + value);
    const measureNames = suffixes.map(value => prefix + 'measure/' + value);
    const marks = [];
    const measures = [];
    let resolveDelivery;
    const delivery = new Promise(resolve => { resolveDelivery = resolve; });
    const observer = new owner.PerformanceObserver(list => resolveDelivery(list));
    observer.observe({entryTypes: ['mark', 'measure']});
    try {
        for (let i = 0; i < names.length; i++) {
            const name = names[i];
            const entry = mark(name, {startTime: i + 1});
            marks.push(entry);
            const constructed = new callee.PerformanceMark(name, {startTime: i + 1});
            const measurement = measure(measureNames[i], {start: name, end: 30});
            measures.push(measurement);
            test('mark preserves name ' + i, () => entry.name === name);
            test('constructor preserves name ' + i, () => constructed.name === name);
            test('constructor snapshot preserves name ' + i, () => json(constructed).name === name);
            test('mark snapshot preserves name ' + i, () => json(entry).name === name);
            test('mark typed query preserves identity ' + i, () => {
                const found = byName(name, 'mark');
                return found.length === 1 && found[0] === entry;
            });
            test('untyped query preserves identity ' + i, () => {
                const found = byName(name);
                return found.length === 1 && found[0] === entry;
            });
            test('measure preserves name ' + i, () => measurement.name === measureNames[i]);
            test('measure snapshot preserves name ' + i, () => json(measurement).name === measureNames[i]);
            test('measure typed query preserves identity ' + i, () => {
                const found = byName(measureNames[i], 'measure');
                return found.length === 1 && found[0] === measurement;
            });
            test('options start resolves exact name ' + i, () => measurement.startTime === i + 1);
            test('options end resolves exact name ' + i, () =>
                measure(prefix + 'options-end', {start: 0, end: name}).duration === i + 1);
            test('legacy start resolves exact name ' + i, () =>
                measure(prefix + 'legacy-start', name, names[0]).startTime === i + 1);
            test('legacy end resolves exact name ' + i, () =>
                measure(prefix + 'legacy-end', names[0], name).duration === i);
        }
        test('snapshot reads native name without invoking own getter', () => {
            const entry = marks[0];
            Object.defineProperty(entry, 'name', {get() { throw Error('author name getter'); }});
            return json(entry).name === names[0];
        });
        test('name coercion preserves UTF-16 and runs before mark options', () => {
            const trace = [];
            const entry = mark({toString() { trace.push('name'); return names[0]; }}, {
                get startTime() { trace.push('time'); return 40; },
            });
            return entry.name === names[0] && trace.join() === 'name,time';
        });
        test('latest mark lookup keeps colliding names separate', () =>
            measure(prefix + 'latest', {start: names[0], end: 50}).startTime === 40 &&
            measure(prefix + 'other', {start: names[1], end: 50}).startTime === 2);
        test('boxed boundary strings preserve exact name', () =>
            measure(prefix + 'boxed', {start: new owner.String(names[1]), end: 10}).startTime === 2);
        test('coerced third argument preserves exact name', () =>
            measure(prefix + 'third', {}, {toString() { return names[1]; }}).duration === 2);
        test('missing surrogate name does not resolve replacement character mark', () =>
            capture(() => measure(prefix + 'missing', {start: prefix + '\ud802', end: 50}))
                instanceof callee.DOMException);
        test('symbol name throws callee TypeError before options getters', () => {
            let reads = 0;
            const error = capture(() => mark(Symbol(), {get startTime() { reads++; return 0; }}));
            return error instanceof callee.TypeError && reads === 0;
        });
        test('query name exception precedes entry type conversion', () => {
            const sentinel = {}; let reads = 0;
            return capture(() => byName({toString() { throw sentinel; }},
                {toString() { reads++; return 'mark'; }})) === sentinel && reads === 0;
        });
        for (const clear of [clearMarks, clearMeasures]) {
            test('clear symbol rejects without clearing entries ' + checks.length, () =>
                capture(() => clear(Symbol())) instanceof callee.TypeError &&
                byName(names[1], 'mark').length === 1 && byName(measureNames[1], 'measure').length === 1);
        }
        // Clear while the observer notification is still queued. Its list must
        // retain the original entries and their names independently of the buffer.
        for (let i = 0; i < names.length; i++) {
            clearMarks({toString() { return names[i]; }});
            test('clearMarks removes only exact name ' + i, () => names.every((name, index) =>
                byName(name, 'mark').length === (index <= i ? 0 : 1)));
            test('clearMarks retains measures ' + i, () => byName(measureNames[i], 'measure')[0] === measures[i]);
        }
        for (let i = 0; i < measureNames.length; i++) {
            clearMeasures({toString() { return measureNames[i]; }});
            test('clearMeasures removes only exact name ' + i, () => measureNames.every((name, index) =>
                byName(name, 'measure').length === (index <= i ? 0 : 1)));
        }
        const list = await delivery;
        const listByName = (...args) => Reflect.apply(
            callee.PerformanceObserverEntryList.prototype.getEntriesByName, list, args);
        for (let i = 0; i < names.length; i++) {
            test('observer mark name identity after clear ' + i, () => {
                const found = listByName(names[i], 'mark');
                return found.length === (i === 0 ? 2 : 1) && found[0] === marks[i];
            });
            test('observer measure name identity after clear ' + i, () => {
                const found = listByName(measureNames[i], 'measure');
                return found.length === 1 && found[0] === measures[i];
            });
        }
        test('observer name coercion runs before type conversion', () => {
            const trace = [];
            const found = listByName({toString() { trace.push('name'); return names[1]; }},
                {toString() { trace.push('type'); return 'mark'; }});
            return trace.join() === 'name,type' && found.length === 1 && found[0] === marks[1];
        });
        test('observer symbol name rejects with callee TypeError', () =>
            capture(() => listByName(Symbol())) instanceof callee.TypeError);
    } finally {
        observer.disconnect();
        for (const name of names) clearMarks(name);
        for (const entry of method('getEntriesByType')('measure')) {
            if (entry.name.startsWith(prefix)) clearMeasures(entry.name);
        }
    }
    return checks;
}
