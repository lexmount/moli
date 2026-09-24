async function navigationTraversalOrigin(target, scenario) {
    const token = `${target}-${scenario}`;
    const remote = new URL(location.href);
    remote.hostname = location.hostname === 'localhost' ? '127.0.0.1' : 'localhost';
    if (target === 'nested') {
        // The srcdoc creator is cross-origin to the top-level host. Its origin
        // must survive serialization without being replaced by the top origin.
        const container = document.createElement('iframe');
        const url = new URL('/compat/child-dynamic-markup-document', remote);
        const script = `(${navigationTraversalOrigin.toString()})('child', 'srcdoc')
            .then(value => parent.postMessage({nestedOriginResult: value}, '*'),
                  error => parent.postMessage({nestedOriginError: error.message}, '*'));`;
        url.searchParams.set('markup', '<!doctype html><body><script>' +
            script.replaceAll('</script>', '<\\/script>') + '</script>');
        let receiveNested, timeout;
        try {
            return await new Promise((resolve, reject) => {
                receiveNested = event => {
                    if (event.source !== container.contentWindow) return;
                    if (event.data?.nestedOriginError) reject(new Error(event.data.nestedOriginError));
                    if (event.data?.nestedOriginResult) resolve(event.data.nestedOriginResult);
                };
                addEventListener('message', receiveNested);
                timeout = setTimeout(() => reject(new Error('nested origin traversal timed out')), 10000);
                container.src = url.href;
                document.body.append(container);
            });
        } finally {
            clearTimeout(timeout);
            removeEventListener('message', receiveNested);
            container.remove();
        }
    }
    let pending, win, frame;
    const messages = [];
    function receive(event) {
        if (event.source !== win || event.data?.token !== token) return;
        messages.push(event.data);
        if (pending && event.data.label === pending.label) pending.resolve();
    }
    addEventListener('message', receive);
    function markup(label) {
        return `<!doctype html><body><script>
          navigation.updateCurrentEntry({state: {label: ${JSON.stringify(label)}}});
          addEventListener('unload', () => {});
          onpageshow = () => (opener || parent).postMessage({
            token: ${JSON.stringify(token)}, label: ${JSON.stringify(label)}, url: location.href
          }, '*');
        </script>`;
    }
    function url(label, crossOrigin = false) {
        const url = new URL('/compat/child-dynamic-markup-document',
            crossOrigin ? remote : location.href);
        url.searchParams.set('markup', markup(label));
        return url.href;
    }
    async function shown(label, action) {
        await new Promise((resolve, reject) => {
            const timeout = setTimeout(() => reject(new Error(`missing pageshow: ${label}`)), 8000);
            pending = {label, resolve: () => { clearTimeout(timeout); resolve(); }};
            action();
        });
        pending = null;
        await new Promise(resolve => setTimeout(resolve, 0));
    }
    try {
        const first = scenario === 'srcdoc' ? 'about:srcdoc' : url('A', scenario === 'cross');
        await shown('A', () => {
            if (target === 'popup') win = open(first);
            else {
                frame = document.createElement('iframe');
                if (scenario === 'srcdoc') frame.srcdoc = markup('A');
                else frame.src = first;
                document.body.append(frame);
                win = frame.contentWindow;
            }
        });
        const originalEntry = scenario === 'cross' ? null : win.navigation.currentEntry;
        const original = originalEntry && {key: originalEntry.key, id: originalEntry.id};
        if (scenario === 'gap') await shown('middle', () => { win.location = url('middle', true); });
        await shown('B', () => { win.location = url('B'); });
        const exposed = win.navigation.entries().some(entry => entry.url === first);
        const events = [], details = [];
        win.navigation.onnavigate = event => {
            events.push('navigate');
            details.push({
                type: event.navigationType, url: event.destination.url === first,
                sameDocument: event.destination.sameDocument,
                key: event.destination.key, id: event.destination.id,
                index: event.destination.index, state: event.destination.getState(),
                cancelable: event.cancelable, canIntercept: event.canIntercept
            });
            event.signal.addEventListener('abort', () => events.push('abort'));
        };
        win.navigation.onnavigateerror = () => events.push('navigateerror');
        win.navigation.onnavigatesuccess = () => events.push('navigatesuccess');
        // Attempts to forge the private origin metadata must not affect eligibility.
        for (const entry of win.navigation.entries()) entry.__lmNavigationEntryOrigin = remote.origin;
        await shown('A', () => win.history.go(scenario === 'gap' ? -2 : -1));
        return {exposed, events, details, returned: messages.at(-1).url === first,
            original};
    } finally {
        removeEventListener('message', receive);
        if (frame) frame.remove();
        else if (win) win.close();
    }
}
