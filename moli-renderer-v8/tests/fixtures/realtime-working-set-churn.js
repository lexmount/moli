(() => {
  document.documentElement.style.margin = '0';
  document.body.style.margin = '0';
  document.body.innerHTML = `<style>
    #churn-feed { width:240px; height:96px; overflow:auto; scrollbar-width:none; }
    #churn-list { width:200px; }
    .churn-row { display:block; width:160px; height:24px; overflow:hidden;
      margin:0; padding:0; border:0; font:16px/24px MoliAhem; }
    #churn-sentinel { width:160px; height:24px; }
  </style><div id="churn-feed"><div id="churn-list"></div><div id="churn-sentinel"></div></div>`;
  const feed = document.getElementById('churn-feed');
  const list = document.getElementById('churn-list');
  const sentinel = document.getElementById('churn-sentinel');
  const nodes = new Map();
  const state = { raf:[], microtasks:[], ro:[], io:[], clicks:[], wheels:[],
    replacements:[], errors:[] };
  let nextId = 0;
  let phase = 'initial';
  function row() {
    const node = document.createElement('div');
    node.id = 'churn-row-' + nextId++;
    node.className = 'churn-row';
    node.textContent = 'ABCD';
    nodes.set(node.id, node);
    return node;
  }
  function frame(label) {
    requestAnimationFrame(() => {
      state.raf.push(label);
      queueMicrotask(() => state.microtasks.push(label));
    });
  }
  for (let i = 0; i < 16; i++) list.append(row());
  addEventListener('error', event => {
    state.errors.push(event.message);
    event.preventDefault();
  });
  const ro = new ResizeObserver(entries => {
    for (const entry of entries) state.ro.push(entry.contentRect.height);
  });
  ro.observe(list);
  const io = new IntersectionObserver(entries => {
    for (const entry of entries) state.io.push(entry.isIntersecting);
  }, { root:feed });
  io.observe(sentinel);
  feed.addEventListener('wheel', event => state.wheels.push(event.deltaY), {passive:true});
  list.addEventListener('click', event => {
    const node = event.target.closest('.churn-row');
    if (!node) return;
    state.clicks.push({id:node.id, connected:node.isConnected, identity:nodes.get(node.id) === node});
    node.textContent = node.textContent === 'ABCD' ? 'WXYZ' : 'ABCD';
    frame('click:' + phase);
  });
  const operations = [
    'append', 'prepend', 'insert-middle', 'remove-head', 'remove-middle',
    'remove-tail', 'tail-to-head', 'head-to-middle', 'replace-same-shape',
    'text', 'width', 'wheel', 'bottom', 'top', 'shrink', 'grow'
  ];
  function apply(name, round) {
    phase = round + ':' + name;
    switch (name) {
      case 'append': list.append(row(), row()); break;
      case 'prepend': list.prepend(row(), row()); break;
      case 'insert-middle': list.insertBefore(row(), list.children[Math.floor(list.children.length / 2)]); break;
      case 'remove-head': list.firstElementChild.remove(); break;
      case 'remove-middle': list.children[Math.floor(list.children.length / 2)].remove(); break;
      case 'remove-tail': list.lastElementChild.remove(); break;
      case 'tail-to-head': list.prepend(list.lastElementChild); break;
      case 'head-to-middle': list.insertBefore(list.firstElementChild, list.children[Math.floor(list.children.length / 2)]); break;
      case 'replace-same-shape': {
        const old = list.children[3];
        const replacement = old.cloneNode(true);
        old.replaceWith(replacement);
        nodes.set(replacement.id, replacement);
        state.replacements.push({id:replacement.id, same:old === replacement, oldConnected:old.isConnected});
        break;
      }
      case 'text': list.firstElementChild.textContent = 'EFGH'; break;
      case 'width': list.firstElementChild.style.width = '192px'; break;
      case 'wheel': break; // The harness supplies an actual wheel packet.
      case 'bottom': feed.scrollTop = (list.children.length + 1) * 24 - 96; break;
      case 'top': feed.scrollTop = 0; break;
      case 'shrink': while (list.children.length > 3) list.lastElementChild.remove(); break;
      case 'grow': for (let i = 0; i < 11; i++) list.append(row()); break;
      default: throw new Error('unknown churn operation: ' + name);
    }
    frame(phase);
  }
  function snapshot() {
    const rect = node => {
      const r = node.getBoundingClientRect();
      return [r.x, r.y, r.width, r.height, node.offsetWidth, node.offsetHeight,
        node.clientWidth, node.clientHeight, node.scrollWidth, node.scrollHeight];
    };
    const hit = document.elementFromPoint(12, 12)?.closest('.churn-row');
    return {
      phase, state,
      rows:Array.from(list.children, node => ({id:node.id, text:node.textContent,
        identity:nodes.get(node.id) === node, box:rect(node)})),
      feed:rect(feed), list:rect(list), sentinel:rect(sentinel),
      scrollTop:feed.scrollTop, hit:hit?.id ?? null
    };
  }
  globalThis.churn = { apply, snapshot, operations, feed, list, state, ro, io };
  frame('initial');
})();
