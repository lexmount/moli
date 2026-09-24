async function navigationWindowReuseProbe(mode, replaceAgain) {
    const frame = document.createElement('iframe');
    const destination = new URL('?first', location.href).href;
    if (mode === 'http') frame.src = destination;
    if (mode === 'srcdoc') frame.srcdoc = '<!doctype html><p>first</p>';
    let loaded = new Promise(resolve => { frame.onload = resolve; });
    document.body.append(frame);
    const w = frame.contentWindow;
    if (mode === 'document-open') {
        w.document.open();
        w.document.write('<p>opened</p>');
        w.document.close();
    }
    if (mode === 'fragment') w.location.hash = 'initial';
    const initial = { navigation: w.navigation, document: w.document, Array: w.Array };
    w.lifetimeMarker = 1;
    const events = [];
    let recordEvents = false;
    initial.navigation.onnavigate = event => {
        if (!recordEvents) return;
        events.push('property');
        event.preventDefault();
    };
    initial.navigation.addEventListener('navigate', event => {
        if (!recordEvents) return;
        events.push('listener');
        event.preventDefault();
    });
    if (mode !== 'http' && mode !== 'srcdoc') {
        loaded = new Promise(resolve => { frame.onload = resolve; });
        frame.src = destination;
    }
    await loaded;
    const firstNavigation = w.navigation;
    const result = {
        sameNavigation: firstNavigation === initial.navigation,
        sameRealm: w.Array === initial.Array,
        documentChanged: w.document !== initial.document,
        marker: w.lifetimeMarker === 1,
        entries: firstNavigation.entries().length,
        currentURL: firstNavigation.currentEntry?.url === w.location.href,
    };
    recordEvents = true;
    w.location.hash = 'navigate';
    result.events = events.slice();
    result.canceled = w.location.hash !== '#navigate';
    if (replaceAgain) {
        recordEvents = false;
        const next = new URL('?second', location.href).href;
        loaded = new Promise(resolve => {
            frame.onload = () => { if (w.document.URL === next) resolve(); };
        });
        frame.src = next;
        frame.removeAttribute('srcdoc');
        await loaded;
        recordEvents = true;
        w.location.hash = 'second-navigation';
        result.second = {
            newNavigation: w.navigation !== firstNavigation,
            newRealm: w.Array !== initial.Array,
            marker: w.lifetimeMarker === 1,
            oldEntries: firstNavigation.entries().length,
            oldCurrentEntry: firstNavigation.currentEntry === null,
            oldEvents: events.slice(result.events.length),
            fragmentCommitted: w.location.hash === '#second-navigation',
        };
    }
    frame.remove();
    return result;
}
