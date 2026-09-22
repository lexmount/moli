async function iframeSrcdocSandboxHistory(before, after, ordinaryURL) {
  const frame = document.createElement('iframe');
  const loaded = () => new Promise(resolve => frame.addEventListener('load', resolve, {once: true}));
  const settle = () => new Promise(resolve => setTimeout(resolve, 0));
  const messages = [];
  const receive = event => { if (event.source === frame.contentWindow) messages.push(event.data); };
  addEventListener('message', receive);
  const navigate = async action => {
    const ready = loaded();
    action();
    await ready;
    await settle();
  };
  try {
    frame.setAttribute('sandbox', before);
    await navigate(() => { frame.src = ordinaryURL; document.body.append(frame); });
    await navigate(() => frame.srcdoc = '<p>historical sandbox source</p>' +
      '<script>parent.postMessage({origin:origin, text:document.querySelector("p").textContent}, "*")<' + '/script>');
    await navigate(() => frame.contentWindow.location.href = ordinaryURL + '&away');
    messages.length = 0;
    frame.setAttribute('sandbox', after);
    await navigate(() => history.back());
    // Messages queued by parser scripts precede the load/next-task boundary.
    return {messages, accessible: frame.contentDocument !== null};
  } finally {
    removeEventListener('message', receive);
    frame.remove();
  }
}
