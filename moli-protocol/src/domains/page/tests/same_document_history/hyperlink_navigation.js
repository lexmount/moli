async function setupClickProbe(target, mode, destination, action = 'cancel') {
    const root = window;
    let owner = root, targetWindow = root, frame;
    document.body.style.margin = '0';
    const childURL = new URL('?child', location.href).href;
    if (target.startsWith('child') || target === 'named-child') {
        frame = document.createElement('iframe'); frame.name = 'click-destination';
        frame.style.cssText = 'position:absolute;left:0;top:0;border:0;width:300px;height:200px';
        frame.src = childURL;
        await new Promise(resolve => {frame.onload = resolve; document.body.append(frame)});
        targetWindow = frame.contentWindow;
        if (target.startsWith('child')) owner = targetWindow;
        if (target === 'child-top' || target === 'child-parent') targetWindow = root;
    } else if (target === 'named-popup') {
        targetWindow = open(childURL, 'click-destination');
        await new Promise(resolve => targetWindow.addEventListener('load', resolve, {once:true}));
    }
    await new Promise(resolve => setTimeout(resolve,0));
    const link = owner.document.createElement('a');
    link.id = 'link'; link.textContent = 'navigate';
    link.style.cssText = 'position:absolute;left:0;top:0;display:block;width:120px;height:80px';
    const url = new URL(targetWindow.location.href);
    if (destination === 'fragment') url.hash = 'click'; else url.search = '?new-document';
    link.href = url.href;
    if (target.startsWith('named')) link.target = 'click-destination';
    if (target === 'child-top') link.target = '_top';
    if (target === 'child-parent') link.target = '_parent';
    owner.document.body.append(link);
    root.records = []; root.clicks = [];
    root.probeTarget = targetWindow; root.probeInitialURL = targetWindow.location.href;
    root.probeInitialDocument = targetWindow.document; root.probeDestination = url.href;
    root.probeSuccess = false;
    targetWindow.navigation.onnavigatesuccess = () => { root.probeSuccess = true; };
    targetWindow.navigation.onnavigate = event => {
        root.records.push({userInitiated:event.userInitiated, source:event.sourceElement === link,
            sourceNull:event.sourceElement === null, sameDocument:event.destination.sameDocument,
            type:event.navigationType, hashChange:event.hashChange});
        if (action === 'cancel') event.preventDefault();
        if (action === 'intercept') event.intercept({handler: () => Promise.resolve()});
    };
    link.onclick = event => {
        root.clicks.push(event.isTrusted);
        if (mode === 'location') {event.preventDefault(); targetWindow.location.href = url.href;}
        if (mode === 'nested-click' && event.isTrusted) {event.preventDefault(); link.click();}
    };
    root.probeOwner = owner; root.probeLink = link;
    if (mode === 'script') link.click();
    if (mode === 'dispatch') link.dispatchEvent(new owner.MouseEvent('click', {bubbles:true,cancelable:true}));
    return {x:20,y:20};
}
