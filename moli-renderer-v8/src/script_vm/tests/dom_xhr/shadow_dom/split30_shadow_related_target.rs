use super::*;
#[test]
fn shadow_dom_event_clears_non_node_related_targets_with_initial_dispatch_path() {
    let mut vm = new_storage_html_test_vm("https://shadow-related-target-cleanup.test/");

    let result = vm
        .eval(
            r#"
(() => {
  let checked = 0;
  for (const mode of ['open', 'closed']) {
    for (const Constructor of [FocusEvent, MouseEvent, PointerEvent, DragEvent]) {
      for (const related of [new EventTarget(), new XMLHttpRequest(), new AbortController().signal, window]) {
        for (const scenario of ['shadow-root', 'shadow-child', 'moved-out', 'light-dom', 'composed-child']) {
          const host = document.body.appendChild(document.createElement('div'));
          const shadow = host.attachShadow({mode});
          const child = shadow.appendChild(document.createElement('span'));
          const target = scenario === 'shadow-root' ? shadow : scenario === 'light-dom' ? host : child;
          const event = new Constructor('related-target-cleanup', {
            bubbles: true, composed: scenario === 'composed-child', relatedTarget: related
          });
          let called = 0;
          target.addEventListener(event.type, current => {
            if (current.target !== target || current.currentTarget !== target ||
                current.relatedTarget !== related || current.eventPhase !== Event.AT_TARGET ||
                current.composedPath().length === 0) {
              throw new Error('incorrect listener state');
            }
            called++;
            if (scenario === 'moved-out') document.body.appendChild(child);
          });
          const allowed = target.dispatchEvent(event);
          const clear = ['shadow-root', 'shadow-child', 'moved-out'].includes(scenario);
          if (!allowed || called !== 1 || event.target !== (clear ? null : host) ||
              event.relatedTarget !== (clear ? null : related) || event.currentTarget !== null ||
              event.eventPhase !== Event.NONE || event.composedPath().length !== 0 ||
              (scenario === 'moved-out' && child.parentNode !== document.body)) {
            throw new Error(`${mode}/${Constructor.name}/${scenario}: incorrect post-dispatch state`);
          }
          child.remove();
          host.remove();
          checked++;
        }
      }
    }
  }
  return checked;
})()
"#,
        )
        .expect("all non-Node related targets should follow the initial shadow dispatch path");

    assert_eq!(result, "160");
}

#[test]
fn shadow_dom_event_related_target_cleanup_uses_private_state() {
    let mut vm = new_storage_html_test_vm("https://shadow-related-target-private-state.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const host = document.body.appendChild(document.createElement('div'));
  const shadow = host.attachShadow({mode: 'closed'});
  const target = shadow.appendChild(document.createElement('span'));
  const related = new EventTarget();
  const event = new FocusEvent('private-related-target', {relatedTarget: related});
  const getter = Object.getOwnPropertyDescriptor(FocusEvent.prototype, 'relatedTarget').get;
  let getterCalls = 0;
  Object.defineProperty(event, 'relatedTarget', {
    get() { getterCalls++; throw new Error('author getter must not run'); }
  });
  let listenerSawRelated = false;
  target.addEventListener(event.type, current => { listenerSawRelated = getter.call(current) === related; });
  target.dispatchEvent(event);
  const ordinary = new Event('ordinary-event');
  target.dispatchEvent(ordinary);
  return [listenerSawRelated, getter.call(event) === null, event.target === null,
    getterCalls, 'relatedTarget' in ordinary].join('|');
})()
"#,
        )
        .expect("shadow cleanup should bypass author getters and preserve ordinary Event shape");

    assert_eq!(result, "true|true|true|0|false");
}
