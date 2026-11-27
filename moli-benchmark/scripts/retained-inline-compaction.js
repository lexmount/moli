// Evaluate through CDP Runtime.evaluate with returnByValue: true against
// `moli serve --realtime`. Alternate baseline/candidate processes and compare
// every geometry array before comparing timings. The height cases exercise
// ordinary invalidation without changing construction IDs.
(() => {
  const cases = [];
  for (const mutation of ["structure", "height"]) {
    for (const count of [64, 256, 1024]) {
      document.head.innerHTML = `<style>
        body { margin: 0; overflow: hidden }
        #prefix > div { height: 3px }
        main { width: 480px }
        p { margin: 0; font: 16px/1.25 sans-serif }
        span { color: #195092 }
      </style>`;
      document.body.innerHTML = '<section id="prefix"><div></div></section><main>' +
        '<p>Unchanged text <span>with nested styles and Hebrew אבג</span> then more words for line wrapping.</p>'.repeat(count) +
        '</main>';
      const prefix = document.getElementById("prefix");
      const main = document.querySelector("main");
      const read = () => {
        const box = main.getBoundingClientRect();
        const last = main.lastElementChild.getBoundingClientRect();
        return [box.x, box.y, box.width, box.height, last.y, last.height];
      };
      read();
      const samples = [];
      const geometry = [];
      for (let i = 0; i < 24; i++) {
        const start = performance.now();
        if (mutation === "structure") {
          if (i % 2 === 0) prefix.prepend(document.createElement("div"));
          else prefix.firstChild.remove();
        } else {
          prefix.firstChild.style.height = i % 2 === 0 ? "6px" : "3px";
        }
        geometry.push(read());
        samples.push(performance.now() - start);
      }
      cases.push({mutation, count, samples, geometry, total: samples.reduce((a, b) => a + b, 0)});
    }
  }
  return {cases};
})()
