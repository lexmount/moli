(() => {
  const afterRendering = () => new Promise(resolve => {
    requestAnimationFrame(() => requestAnimationFrame(() => setTimeout(resolve, 0)));
  });
  const cases = [
    {
      name: 'IntersectionObserver explicit roots require a live Document, not an ancestor relation',
      expected: 'false|false|false||;|false|false||',
      async run() {
        const detached = document.implementation.createHTMLDocument('');
        const targets = [document.createElement('div'), detached.createElement('div')];
        detached.body.appendChild(targets[1]);
        const roots = [null, document, document.createElement('div'), detached,
                       detached.createElement('div')];
        const results = targets.map(() => roots.map(() => []));
        const observers = targets.flatMap((target, targetIndex) => roots.map((root, index) => {
          const observer = new IntersectionObserver(entries => {
            results[targetIndex][index].push(...entries.map(entry => entry.isIntersecting));
          }, {root});
          observer.observe(target);
          return observer;
        }));
        try {
          await afterRendering();
          return results.map(roots => roots.map(entries => entries.join(',')).join('|')).join(';');
        } finally { observers.forEach(observer => observer.disconnect()); }
      }
    },
    {
      name: 'IntersectionObserver distinguishes disconnected and windowless targets through adoption',
      expected: 'live:false|live:true,select:true,div:true|removed:true',
      async run() {
        const detached = document.implementation.createHTMLDocument('');
        const targets = [document.createElement('div'), detached.createElement('select'),
                         detached.createElement('div')];
        const names = ['live', 'select', 'div'];
        for (const target of targets) target.style.cssText = 'display:block;width:40px;height:20px';
        // A parent in a windowless document does not make the select observable.
        detached.body.appendChild(targets[1]);
        const entries = [];
        const observer = new IntersectionObserver(batch => {
          for (const entry of batch) {
            const index = targets.indexOf(entry.target);
            entries.push(`${names[index] || 'wrong identity'}:${entry.isIntersecting}`);
          }
        });
        try {
          targets.forEach(target => observer.observe(target));
          await afterRendering();
          const initial = entries.splice(0).join(',');
          for (const target of targets) {
            document.adoptNode(target);
            document.body.appendChild(target);
          }
          await afterRendering();
          const adopted = entries.splice(0).join(',');
          targets.forEach(target => observer.unobserve(target));
          targets.forEach(target => target.remove());
          await afterRendering();
          return `${initial}|${adopted}|removed:${entries.length === 0}`;
        } finally {
          observer.disconnect();
          targets.forEach(target => target.remove());
        }
      }
    },
    {
      name: 'ResizeObserver samples adopted native proxy elements and preserves entry identity',
      expected: 'true:40x20|true:60x30|removed:true',
      async run() {
        const target = document.implementation.createHTMLDocument('').createElement('select');
        target.style.cssText = 'display:block;box-sizing:content-box;border:0;padding:0;width:40px;height:20px';
        document.adoptNode(target);
        document.body.appendChild(target);
        const entries = [];
        const observer = new ResizeObserver(batch => {
          for (const entry of batch) {
            entries.push(`${entry.target === target}:${entry.contentRect.width}x${entry.contentRect.height}`);
          }
        });
        try {
          observer.observe(target);
          await afterRendering();
          const initial = entries.splice(0).join(',');
          target.style.width = '60px';
          target.style.height = '30px';
          await afterRendering();
          const resized = entries.splice(0).join(',');
          observer.unobserve(target);
          target.style.width = '80px';
          await afterRendering();
          return `${initial}|${resized}|removed:${entries.length === 0}`;
        } finally {
          observer.disconnect();
          target.remove();
        }
      }
    }
  ];
  globalThis.__observerDetached = {
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
