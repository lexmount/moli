(async () => {
  document.body.innerHTML = '<div id="pane" style="overflow:auto;width:100px;height:100px"><div id="content" style="height:600px"></div></div>';
  const pane = document.getElementById('pane');
  const content = document.getElementById('content');
  pane.getBoundingClientRect();
  await new Promise(resolve => requestAnimationFrame(resolve));
  await new Promise(resolve => requestAnimationFrame(resolve));
  const rows = [];
  let count = 0;
  // After initialization only native scroll notifications drive the next
  // DOM update and scroll. No timer, rAF, capture, or polling advances it.
  await new Promise(resolve => {
    pane.addEventListener('scroll', event => {
      ++count;
      rows.push(['scroll', pane.scrollTop, event.bubbles, event.cancelable,
                 event.target === pane]);
      content.style.height = (600 + count * 100) + 'px';
      if (count < 3) pane.scrollTop += 40;
    });
    pane.addEventListener('scrollend', event => {
      rows.push(['scrollend', pane.scrollTop, event.bubbles, event.cancelable,
                 event.target === pane]);
      if (count === 3) resolve();
    });
    pane.scrollTop = 40;
  });
  return { rows, failures: [] };
})()
