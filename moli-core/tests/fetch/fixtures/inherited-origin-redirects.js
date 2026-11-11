async function(cross, kinds = ['blank', 'srcdoc', 'srcdoc-base']) {
    for (const cookie of ['strict=1; SameSite=Strict', 'lax=1; SameSite=Lax', 'default=1']) {
        document.cookie = cookie + '; Path=/';
    }
    const home = location.origin;
    const results = {};
    for (const kind of kinds) {
        const frame = document.createElement('iframe');
        if (kind !== 'blank') {
            frame.srcdoc = kind === 'srcdoc-base'
                ? `<base href="${cross}/"><body>redirect probe</body>`
                : '<body>redirect probe</body>';
        }
        const loaded = new Promise(resolve => frame.onload = resolve);
        document.body.append(frame);
        await loaded;
        const completed = new Promise(resolve => {
            const handler = event => {
                if (event.source !== frame.contentWindow || event.data.kind !== kind) return;
                removeEventListener('message', handler);
                resolve(event.data.results);
            };
            addEventListener('message', handler);
        });
        // Inject into the existing blank Document; document.open/write would
        // replace its URL and hide the absent-referrer regression.
        const script = frame.contentDocument.createElement('script');
        script.textContent = `
            (async () => {
                const results = {documentURL: document.URL};
                for (const api of ['fetch', 'xhr', 'sync-xhr']) {
                    const query = 'kind=' + ${JSON.stringify(kind)} + '&api=' + api;
                    const target = ${JSON.stringify(home)} + '/same-origin-target?' + query;
                    const url = ${JSON.stringify(home)} + '/redirect-allowed?' + query
                        + '&to=' + encodeURIComponent(target);
                    try {
                        if (api === 'fetch') {
                            results[api] = await (await fetch(url, {
                                credentials: 'include', headers: {'X-Probe': 'yes'}
                            })).text();
                        } else {
                            results[api] = await new Promise(resolve => {
                                const xhr = new XMLHttpRequest();
                                xhr.open('GET', url, api !== 'sync-xhr');
                                xhr.withCredentials = true;
                                xhr.setRequestHeader('X-Probe', 'yes');
                                xhr.onload = () => resolve(xhr.responseText);
                                xhr.onerror = () => resolve('error');
                                try {
                                    xhr.send();
                                    if (api === 'sync-xhr') resolve(xhr.responseText);
                                } catch (error) { resolve(error.name); }
                            });
                        }
                    } catch (error) { results[api] = error.name; }
                }
                parent.postMessage({kind: ${JSON.stringify(kind)}, results}, '*');
            })();`;
        frame.contentDocument.body.append(script);
        results[kind] = await completed;
        frame.remove();
    }
    return JSON.stringify(results);
}
