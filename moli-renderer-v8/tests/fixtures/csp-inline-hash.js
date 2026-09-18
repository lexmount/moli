(() => {
  const source = 'globalThis.__inlineHashRuns += 1;/*☃ 8*/';
  globalThis.__inlineHashRuns = 0;
  const root = document.body || document.documentElement;
  const counts = [];
  const script = text => {
    const element = document.createElement('script');
    element.text = text;
    root.appendChild(element);
    element.remove();
    counts.push(globalThis.__inlineHashRuns);
  };
  const handler = text => {
    const element = document.createElement('button');
    element.setAttribute('onclick', text);
    root.appendChild(element);
    element.click();
    element.remove();
    counts.push(globalThis.__inlineHashRuns);
  };
  script(source);
  handler(source);
  script(source + ' ');
  handler(source + ' ');
  return counts;
})()
