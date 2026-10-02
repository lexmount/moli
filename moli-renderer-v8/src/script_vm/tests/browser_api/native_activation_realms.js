(() => {
  const NativeObject = Object;
  const child = document.getElementById('child').contentWindow;
  const realms = [window, child].map(global => ({
    global, document: global.document, Ui: global.UIEvent, Event: global.Event,
    Mouse: global.MouseEvent, click: global.HTMLElement.prototype.click
  }));
  const windowless = realms.map(realm => realm.document.implementation.createHTMLDocument(''));
  const rows = [], restores = [], actions = [];
  let reads = 0;
  function observe(node, type, realm, view, trusted, label, after) {
    let captured;
    node.addEventListener(type, event => { captured = event; }, {capture: true});
    node.addEventListener(type, event => {
      const opposite = realms[1 - realms.indexOf(realm)];
      const checks = {
        realm: event instanceof realm.Event && !(event instanceof opposite.Event),
        ui: type === 'input' || type === 'change'
          ? !(event instanceof realm.Ui) : event instanceof realm.Ui,
        identity: event === captured,
        target: event.target === node && event.currentTarget === node,
        type: event.type === type,
        trusted: event.isTrusted === trusted
      };
      if (type !== 'input' && type !== 'change') {
        NativeObject.defineProperty(checks, 'view', {value: event.view === view, enumerable: true});
      }
      rows.push({label, type, checks});
      if (after) after(event);
    });
  }
  function makeNode(creationDocument, targetDocument, tag = 'button') {
    const node = creationDocument.createElement(tag);
    if (tag === 'input') node.type = 'checkbox';
    targetDocument.body.appendChild(node);
    return node;
  }
  for (const creation of realms) {
    for (const target of realms) {
      for (const caller of realms) {
        const label = `create ${realms.indexOf(creation)} / target ${realms.indexOf(target)} / caller ${realms.indexOf(caller)}`;
        const button = makeNode(creation.document, target.document);
        observe(button, 'click', target, target.global, false, `button ${label}`);
        actions.push(() => caller.click.call(button));
        const checkbox = makeNode(creation.document, target.document, 'input');
        observe(checkbox, 'click', target, target.global, false, `checkbox ${label}`);
        for (const type of ['input', 'change']) {
          observe(checkbox, type, target, undefined, true, `checkbox ${label}`, () => {
            rows[rows.length - 1].checks.checked = checkbox.checked;
          });
        }
        actions.push(() => caller.click.call(checkbox));
      }
    }
  }
  for (const target of realms) {
    const doc = windowless[realms.indexOf(target)];
    for (const caller of realms) {
      const button = makeNode(doc, doc);
      observe(button, 'click', target, null, false, `windowless ${realms.indexOf(target)} / caller ${realms.indexOf(caller)}`);
      actions.push(() => caller.click.call(button));
    }
    const caller = realms[1 - realms.indexOf(target)];
    const canceled = makeNode(target.document, target.document, 'input');
    observe(canceled, 'click', target, target.global, false, 'canceled checkbox', event => event.preventDefault());
    canceled.addEventListener('input', () => rows.push({label: 'unexpected canceled input', checks: {absent: false}}));
    canceled.addEventListener('change', () => rows.push({label: 'unexpected canceled change', checks: {absent: false}}));
    actions.push(() => {
      caller.click.call(canceled);
      rows[rows.length - 1].checks.rollback = !canceled.checked;
    });
    const moved = makeNode(target.document, target.document, 'input');
    observe(moved, 'click', target, target.global, false, 'adopt in click', () => caller.document.body.appendChild(moved));
    observe(moved, 'input', caller, undefined, true, 'adopt in input', () => target.document.body.appendChild(moved));
    observe(moved, 'change', target, undefined, true, 'change after second adoption');
    actions.push(() => caller.click.call(moved));
  }
  for (const author of realms) {
    for (const target of realms) {
      const button = makeNode(target.document, target.document);
      const synthetic = new author.Mouse('probe', {view: child});
      observe(button, 'probe', author, child, false, 'author event keeps construction realm', event => {
        rows[rows.length - 1].checks.original = event === synthetic;
      });
      actions.push(() => button.dispatchEvent(synthetic));
    }
    const target = realms[1 - realms.indexOf(author)];
    const checkbox = makeNode(target.document, target.document, 'input');
    const synthetic = new author.Mouse('click', {bubbles: true, view: author.global});
    observe(checkbox, 'click', author, author.global, false, 'author click keeps construction realm', event => {
      rows[rows.length - 1].checks.original = event === synthetic;
    });
    for (const type of ['input', 'change']) observe(checkbox, type, target, undefined, true, 'author click native default');
    actions.push(() => checkbox.dispatchEvent(synthetic));
  }
  const outer = makeNode(child.document, child.document);
  const inner = makeNode(document, document);
  observe(inner, 'click', realms[0], window, false, 'reentrant parent activation');
  observe(outer, 'click', realms[1], child, false, 'reentrant child activation', event => {
    const saved = event;
    realms[1].click.call(inner);
    rows[rows.length - 2].checks.retained = window.event === saved &&
      event.currentTarget === outer && event.view === child && event instanceof realms[1].Ui;
  });
  actions.push(() => realms[0].click.call(outer));
  function poison(object, name) {
    restores.push([object, name, NativeObject.getOwnPropertyDescriptor(object, name)]);
    NativeObject.defineProperty(object, name, {configurable: true, get() {
      reads++;
      throw Error('author native activation hook');
    }});
  }
  for (const realm of realms) {
    for (const name of ['Event', 'UIEvent', 'MouseEvent', 'PointerEvent']) poison(realm.global, name);
    for (const name of ['view', 'detail']) poison(realm.global.Object.prototype, name);
  }
  try {
    for (const action of actions) action();
  } finally {
    for (let i = restores.length - 1; i >= 0; i--) {
      const [object, name, descriptor] = restores[i];
      if (descriptor) NativeObject.defineProperty(object, name, descriptor);
      else delete object[name];
    }
  }
  const passed = rows.length === 56 && reads === 0 &&
    rows.every(row => NativeObject.values(row.checks).every(value => value === true));
  globalThis.__uiEventResults = {rows, reads, expectedRows: 56, passed};
  return passed;
})()
