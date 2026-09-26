(() => {
  const cases = [
    {
      name: 'observation order survives duplicates, removals, DOM reordering and disconnect',
      expected: '8,3,10,0,9,2,11,1,6,4,7,5:111111111111|8,10,0,9,2,11,1,4,7,5,3,6:000000000000|8,10,0,9,2,11,1,4,7,5,3,6:111111111111|6,8,0:111',
      run: () => new Promise(resolve => {
        const targets = Array.from({length: 12}, (_, index) => {
          const target = document.body.appendChild(document.createElement('div'));
          target.id = String(index);
          target.style.cssText = 'width:20px;height:20px';
          return target;
        });
        const log = [];
        const observer = new IntersectionObserver(entries => {
          log.push(entries.map(entry => entry.target.id).join(',') + ':' +
            entries.map(entry => Number(entry.isIntersecting)).join(''));
          if (log.length === 1) {
            targets.forEach(target => { target.style.display = 'none'; });
            observer.unobserve(targets[3]);
            observer.unobserve(targets[6]);
            observer.observe(targets[3]);
            observer.observe(targets[6]);
          } else if (log.length === 2) {
            document.body.prepend(targets[5]);
            targets.forEach(target => { target.style.display = 'block'; });
            observer.observe(targets[8]);
            observer.observe(targets[6]);
          } else if (log.length === 3) {
            observer.disconnect();
            [6, 8, 0].forEach(index => observer.observe(targets[index]));
          } else {
            observer.disconnect();
            resolve(log.join('|'));
          }
        });
        [8, 3, 10, 0, 9, 2, 11, 1, 6, 4, 7, 5].forEach(index => observer.observe(targets[index]));
        observer.observe(targets[3]);
      })
    },
    {
      name: 'callbacks honor pending queue reads and cancellation for later observers',
      expected: 'own:0|drain:b,a,c|unobserve-drain:a,c|disconnect-drain:|microtask-drain:b,a,c|unobserve:a,c|later:1:b,a,c',
      run: () => new Promise(resolve => {
        const targets = ['a', 'b', 'c'].map(id => {
          const target = document.body.appendChild(document.createElement('div'));
          target.id = id;
          target.style.cssText = 'width:20px;height:20px';
          return target;
        });
        const ids = entries => entries.map(entry => entry.target.id).join(',');
        const log = [];
        const subjects = [];
        let frame = 0;
        const controller = new IntersectionObserver(() => {
          log.push('own:' + controller.takeRecords().length);
          controller.disconnect();
          for (const [mode, observer] of subjects) {
            // Preserve Moli's existing cancellation behavior, also used by
            // Chromium; the spec discussion remains open in IO issue #356.
            if (mode.startsWith('unobserve')) observer.unobserve(targets[1]);
            if (mode.startsWith('disconnect')) observer.disconnect();
            if (mode === 'microtask-drain') {
              queueMicrotask(() => log.push(mode + ':' + ids(observer.takeRecords())));
            } else if (mode.includes('drain')) {
              log.push(mode + ':' + ids(observer.takeRecords()));
            }
          }
          const later = new IntersectionObserver(entries => {
            log.push('later:' + frame + ':' + ids(entries));
            later.disconnect();
          });
          [1, 0, 2].forEach(index => later.observe(targets[index]));
          requestAnimationFrame(() => {
            frame = 1;
            requestAnimationFrame(() => {
              subjects.forEach(([, observer]) => observer.disconnect());
              later.disconnect();
              resolve(log.join('|'));
            });
          });
        });
        for (const mode of ['drain', 'microtask-drain', 'unobserve-drain', 'disconnect-drain', 'unobserve', 'disconnect']) {
          const observer = new IntersectionObserver(entries => {
            log.push(mode + ':' + ids(entries));
            observer.disconnect();
          });
          subjects.push([mode, observer]);
          [1, 0, 2].forEach(index => observer.observe(targets[index]));
        }
        controller.observe(targets[0]);
      })
    }
  ];
  globalThis.__intersectionQueues = {
    result: null,
    expected: null,
    start(index) {
      document.body.replaceChildren();
      const test = cases[index];
      this.result = null;
      this.expected = test.expected;
      test.run().then(result => { this.result = result; });
      return test.name;
    },
    snapshot() {
      return {actual: this.result, expected: this.expected};
    }
  };
  return cases.length;
})();
