async function() {
    const registration = await navigator.serviceWorker.register('/sw.js');
    await navigator.serviceWorker.ready;
    if (!navigator.serviceWorker.controller) {
        await new Promise(resolve => navigator.serviceWorker.addEventListener(
            'controllerchange', resolve, {once: true}));
    }
    for (const cookie of ['strict=1; SameSite=Strict', 'lax=1; SameSite=Lax', 'default=1']) {
        document.cookie = cookie + '; Path=/';
    }
    const home = location.origin;
    const popup = window.open('about:blank');
    try {
        const controlled = await (await popup.fetch(home + '/sw-probe')).text();
        const direct = await (await popup.fetch(home + '/network-final?direct', {
            credentials: 'include'
        })).text();
        const target = home + '/network-final?redirect';
        const redirect = await (await popup.fetch(home + '/sw-redirect?to=' + encodeURIComponent(target), {
            credentials: 'include'
        })).text();
        const seen = await (await fetch('/sw-log')).json();
        return JSON.stringify({documentURL: popup.document.URL, controlled, direct, redirect, seen});
    } finally {
        popup.close();
        await registration.unregister();
    }
}
