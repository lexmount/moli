async function(cross) {
    const frame = document.createElement('iframe');
    const created = new Promise(resolve => {
        const handler = event => {
            if (event.source !== frame.contentWindow || !event.data.blob) return;
            removeEventListener('message', handler);
            resolve(event.data);
        };
        addEventListener('message', handler);
    });
    frame.src = cross + '/blob-creator';
    document.body.append(frame);
    const foreign = await created;
    const local = URL.createObjectURL(new Blob(['local']));
    const cases = {foreign: foreign.blob, local, data: 'data:text/plain,data'};
    const results = {creator: foreign.text};
    for (const [name, url] of Object.entries(cases)) {
        try {
            results['window-' + name] = await (await fetch(url, {mode: 'same-origin'})).text();
        } catch (error) { results['window-' + name] = error.name; }
    }
    const script = URL.createObjectURL(new Blob([`
        onmessage = async event => {
            const results = {};
            for (const [name, url] of Object.entries(event.data)) {
                try {
                    results['worker-' + name] = await (await fetch(url, {mode: 'same-origin'})).text();
                } catch (error) { results['worker-' + name] = error.name; }
            }
            postMessage(results);
        };
    `], {type: 'text/javascript'}));
    const worker = new Worker(script);
    const workerResults = new Promise((resolve, reject) => {
        worker.onmessage = event => resolve(event.data);
        worker.onerror = reject;
    });
    worker.postMessage(cases);
    Object.assign(results, await workerResults);
    worker.terminate();
    URL.revokeObjectURL(script);
    URL.revokeObjectURL(local);
    frame.remove();
    return JSON.stringify(results);
}
