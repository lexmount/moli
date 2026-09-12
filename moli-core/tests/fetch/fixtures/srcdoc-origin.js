async function(cross) {
    const results = {};
    const local = URL.createObjectURL(new Blob(['local']));
    for (const opaque of [false, true]) {
        const kind = opaque ? 'opaque' : 'inherited';
        const cases = [
            {name: 'relative', url: './forbidden-' + kind, mode: 'same-origin'},
            {name: 'home', url: location.origin + (opaque ? '/forbidden-opaque-home' : '/allowed-home'), mode: 'same-origin'},
            {name: 'redirect', url: location.origin + '/redirect-allowed?to=' + encodeURIComponent(cross + '/forbidden-redirect-' + kind), mode: 'same-origin'},
            {name: 'blob', url: local, mode: 'same-origin'},
            {name: 'data', url: 'data:text/plain,data', mode: 'same-origin'},
            {name: 'cors', url: './cors-' + kind, mode: 'cors'},
            {name: 'preflight', url: './preflight-' + kind, mode: 'cors', method: 'PUT', headers: {'X-Probe': 'yes'}}
        ];
        const frame = document.createElement('iframe');
        if (opaque) frame.setAttribute('sandbox', 'allow-scripts');
        const terminal = new Promise(resolve => {
            const handler = event => {
                if (event.source !== frame.contentWindow || event.data.kind !== kind) return;
                removeEventListener('message', handler);
                resolve(event.data.results);
            };
            addEventListener('message', handler);
        });
        frame.srcdoc = `<base href='${cross}/'><script>
            (async () => {
                const results = {};
                for (const test of ${JSON.stringify(cases)}) {
                    try {
                        results[test.name] = await (await fetch(test.url, test)).text();
                    } catch (error) { results[test.name] = error.name; }
                }
                parent.postMessage({kind: '${kind}', results}, '*');
            })();
        <\/script>`;
        document.body.append(frame);
        results[kind] = await terminal;
        frame.remove();
    }
    URL.revokeObjectURL(local);
    const base = document.createElement('base');
    base.href = cross + '/';
    document.head.append(base);
    try {
        await fetch('./forbidden-top-base', {mode: 'same-origin'});
        results.topBase = 'unexpected-success';
    } catch (error) { results.topBase = error.name; }
    base.remove();
    return JSON.stringify(results);
}
