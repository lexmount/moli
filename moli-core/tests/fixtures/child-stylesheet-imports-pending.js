async function(kind) {
  const token = 'delayed-' + kind;
  const key = token + ':/child-import-branch.css';
  const frame = document.createElement('iframe');
  let loaded = false;
  let parserMessages = 0;
  const parsed = new Promise(resolve => {
    window.addEventListener('message', function listener(event) {
      if (!event.data || !event.data.childImportProbe || !event.data.href.includes('token=' + token)) return;
      window.removeEventListener('message', listener);
      parserMessages++;
      resolve();
    });
  });
  const load = new Promise(resolve => {
    frame.onload = () => { loaded = true; resolve(); };
  });
  frame.src = '/child-import-page?kind=' + kind + '&token=' + token + '&scenario=delayed';
  document.body.append(frame);
  await fetch('/probe-started?token=' + encodeURIComponent(key));
  const before = {loaded, parserMessages};
  await fetch('/probe-release?token=' + encodeURIComponent(key));
  await Promise.all([load, parsed]);
  return {before, loaded, parserMessages, snapshot: frame.contentWindow.parserSheetSnapshot};
}
