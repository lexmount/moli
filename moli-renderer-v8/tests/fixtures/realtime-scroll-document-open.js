(async () => {
  const settle = async () => {
    for (let i = 0; i < 4; ++i)
      await new Promise(resolve => requestAnimationFrame(resolve));
  };
  const markup = '<!doctype html><body style="height:3000px"></body>';
  document.body.style.height = '3000px';
  await settle();
  let events = [];
  const rows = [];
  scrollTo(0, 100);
  document.open(); document.write(markup); document.close();
  document.addEventListener('scroll', () => events.push(['scroll', scrollY]));
  document.addEventListener('scrollend', () => events.push(['scrollend', scrollY]));
  await settle();
  rows.push(['before-delivery', events]); events = [];
  document.addEventListener('scroll', () => {
    document.open(); document.write(markup); document.close();
    document.addEventListener('scrollend', () => events.push(['current-end', scrollY]));
    Promise.resolve().then(() => events.push(['microtask']));
  }, { once: true });
  scrollTo(0, 200);
  await settle();
  rows.push(['during-delivery', events]);
  return { rows, failures: [] };
})()
