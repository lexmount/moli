use super::*;

#[test]
fn structured_clone_preserves_native_rectangle_kind_values_and_graph_identity() {
    let mut vm = new_storage_test_vm("https://rect-clone.test/");
    assert_eq!(vm.eval(r#"
(() => {
  const check = (ok, message) => { if (!ok) throw Error(message); };
  if (!document.documentElement) document.appendChild(document.createElement('html'));
  if (!document.body) document.documentElement.appendChild(document.createElement('body'));
  const iframe = document.createElement('iframe');
  document.body.appendChild(iframe);
  const other = iframe.contentWindow;
  for (const realm of [window, other]) {
    for (const name of ['DOMRect', 'DOMRectReadOnly']) {
      const C = realm[name];
      const prototype = C.prototype;
      const values = [-0, NaN, Infinity, -Infinity];
      const rect = new C(...values);
      Object.defineProperty(rect, 'x', { enumerable: true, get() { throw Error('public x must not be read'); } });
      Object.defineProperty(rect, 'toJSON', { enumerable: true, get() { throw Error('toJSON must not be read'); } });
      const subclass = new (class extends C {})(1, 2, -3, -4);
      realm[name] = function PoisonedConstructor() { throw Error('public constructor must not be called'); };
      try {
        const graph = realm.structuredClone({ rect, again: rect, list: [rect], subclass });
        const clone = graph.rect;
        check(clone !== rect && graph.again === clone && graph.list[0] === clone, name + ' cloned graph identity');
        check(Object.getPrototypeOf(clone) === prototype, name + ' realm-local native prototype');
        check(Object.keys(clone).length === 0, name + ' ignores expandos');
        for (const [i, key] of ['x', 'y', 'width', 'height'].entries())
          check(Object.is(clone[key], values[i]), name + ' preserves ' + key);
        check(Object.getPrototypeOf(graph.subclass) === prototype, name + ' strips author subclass prototype');
        check(graph.subclass.x === 1 && graph.subclass.y === 2 && graph.subclass.width === -3 && graph.subclass.height === -4, name + ' subclass native state');
        if (name === 'DOMRect') {
          clone.x = 10;
          check(clone.x === 10, name + ' mutable clone');
        } else {
          check(Reflect.set(clone, 'x', 10) === false && Object.is(clone.x, -0), name + ' readonly clone');
        }
      } finally { realm[name] = C; }
    }
  }
  const source = new other.DOMRect(5, 6, -7, -8);
  const cross = structuredClone(source);
  check(Object.getPrototypeOf(cross) === DOMRect.prototype && cross.x === 5 && cross.width === -7, 'cross-realm reconstruction');
  let traps = 0;
  let error;
  try { structuredClone(new Proxy(source, { ownKeys() { traps++; return []; } })); } catch (caught) { error = caught; }
  check(error instanceof DOMException && error.name === 'DataCloneError' && traps === 0, 'author Proxy rejection');
  history.replaceState({ rect: new DOMRectReadOnly(-0, 2, -3, 4) }, '', '#rectangle');
  const stored = history.state.rect;
  check(Object.getPrototypeOf(stored) === DOMRectReadOnly.prototype && Object.is(stored.x, -0) && stored.width === -3, 'history storage reconstruction');
  return 'passed';
})()
"#).unwrap(), "passed");
}
