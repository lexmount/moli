globalThis.__imageDocumentNavigation = {complete: false, checks: []};
globalThis.__imageDocumentExecuted = 0;
globalThis.__imageDocumentNavigationDone = (async () => {
  const result = __imageDocumentNavigation;
  const check = (name, passed) => result.checks.push({name, passed: !!passed});
  const gif = Uint8Array.from(atob('R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7'), c => c.charCodeAt(0));
  for (const broken of [false, true]) {
    const url = URL.createObjectURL(new Blob([
      broken ? '<script>parent.__imageDocumentExecuted++</script>' : gif
    ], {type: broken ? 'image/png' : 'image/gif'}));
    const frame = document.createElement('iframe');
    const loaded = new Promise(resolve => frame.onload = resolve);
    frame.src = url; document.body.appendChild(frame); await loaded;
    const doc = frame.contentDocument, image = doc?.body?.firstElementChild;
    check(`${broken}:document`, doc?.URL === url && doc.contentType === (broken ? 'image/png' : 'image/gif') && doc.characterSet === 'UTF-8');
    check(`${broken}:native host`, image instanceof frame.contentWindow.HTMLImageElement && image.localName === 'img');
    check(`${broken}:source`, image?.src === url && image.currentSrc === url && image.complete);
    check(`${broken}:inert`, __imageDocumentExecuted === 0 && !doc.querySelector('script'));
    check(`${broken}:metadata`, image?.naturalWidth === (broken ? 0 : 1) && image?.naturalHeight === (broken ? 0 : 1));
    const promise = image.decode(); let outcome = 'fulfilled';
    try { await promise; } catch (error) { outcome = error.name; }
    check(`${broken}:decode`, outcome === (broken ? 'EncodingError' : 'fulfilled') && Object.getPrototypeOf(promise) === frame.contentWindow.Promise.prototype);
    if (!broken) {
      const replacement = URL.createObjectURL(new Blob([gif], {type: 'image/gif'}));
      image.src = replacement;
      let after = 'fulfilled'; try { await image.decode(); } catch (error) { after = error.name; }
      check('ordinary mutation', after === (globalThis.__imageDocumentDenyImages ? 'EncodingError' : 'fulfilled'));
      URL.revokeObjectURL(replacement);
    }
    frame.remove(); URL.revokeObjectURL(url);
  }
  result.complete = true;
  return result;
})().catch(error => {
  __imageDocumentNavigation.error = String(error);
  __imageDocumentNavigation.complete = true;
  return __imageDocumentNavigation;
});
