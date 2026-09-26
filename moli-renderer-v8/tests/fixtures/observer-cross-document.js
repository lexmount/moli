(() => {
  const afterRendering = realm => new Promise(resolve => {
    realm.requestAnimationFrame(() => realm.requestAnimationFrame(() => realm.setTimeout(resolve, 0)));
  });
  const cases = [
    {
      name: 'opener IntersectionObserver follows popup insertion and visibility changes',
      expected: 'main:true,popup:false|popup:true|popup:false|main:true',
      async run() {
        const popup = window.open('about:blank', '', 'popup');
        const main = document.body.appendChild(document.createElement('div'));
        const target = popup.document.createElement('div');
        main.style.cssText = target.style.cssText = 'width:40px;height:20px';
        const entries = [];
        const observer = new IntersectionObserver(batch => {
          entries.push(...batch.map(entry =>
            `${entry.target === main ? 'main' : entry.target === target ? 'popup' : 'wrong identity'}:${entry.isIntersecting}`));
        });
        try {
          observer.observe(main);
          observer.observe(target);
          await afterRendering(popup);
          await afterRendering(window);
          const initial = entries.splice(0).join(',');
          popup.document.body.appendChild(target);
          await afterRendering(popup);
          const inserted = entries.splice(0).join(',');
          target.style.display = 'none';
          await afterRendering(popup);
          const hidden = entries.splice(0).join(',');
          observer.unobserve(target);
          popup.close();
          observer.unobserve(main);
          observer.observe(main);
          await afterRendering(window);
          return [initial, inserted, hidden, entries.join(',')].join('|');
        } finally {
          observer.disconnect();
          popup.close();
          main.remove();
        }
      }
    },
    ...['opener', 'iframe'].map(owner => ({
      name: `${owner} ResizeObserver samples both top-level Documents without losing geometry`,
      expected: 'main:80:80,popup:40:40|main:90:90,popup:60:60|90:60',
      async run() {
        const popup = window.open('about:blank', '', 'popup');
        const main = document.body.appendChild(document.createElement('div'));
        const target = popup.document.body.appendChild(popup.document.createElement('div'));
        main.style.cssText = 'width:80px;height:20px';
        target.style.cssText = 'width:40px;height:20px';
        const entries = [];
        const frame = document.body.appendChild(document.createElement('iframe'));
        const realm = owner === 'iframe' ? frame.contentWindow : window;
        const observer = new realm.ResizeObserver(batch => {
          entries.push(...batch.map(entry =>
            `${entry.target === main ? 'main' : entry.target === target ? 'popup' : 'wrong identity'}:${entry.contentRect.width}:${entry.target.getBoundingClientRect().width}`));
        });
        try {
          observer.observe(main);
          observer.observe(target);
          await afterRendering(popup);
          await afterRendering(window);
          const initial = entries.splice(0).join(',');
          main.style.width = '90px';
          target.style.width = '60px';
          await afterRendering(popup);
          await afterRendering(window);
          const resized = entries.splice(0).join(',');
          const geometry = `${main.getBoundingClientRect().width}:${target.getBoundingClientRect().width}`;
          return [initial, resized, geometry].join('|');
        } finally {
          observer.disconnect();
          popup.close();
          frame.remove();
          main.remove();
        }
      }
    }))
  ];
  globalThis.__observerCrossDocument = {
    result: null,
    expected: null,
    start(index) {
      const test = cases[index];
      this.result = null;
      this.expected = test.expected;
      test.run().then(result => { this.result = result; }, error => { this.result = String(error); });
      return test.name;
    },
    snapshot() { return {actual: this.result, expected: this.expected}; }
  };
  return cases.length;
})();
