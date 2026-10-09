use super::*;

#[test]
fn animation_transition_and_blob_events_preserve_native_payloads_and_conversions() {
    let mut vm = new_parsed_test_vm("https://value-events.test/", "<!doctype html><body></body>");
    let result = vm
        .eval(include_str!("value_events_v2.js"))
        .expect("event payload constructors should preserve Web IDL semantics");
    assert_eq!(result, "true");
}

#[test]
fn page_swap_event_uses_nullable_navigation_and_transition_payloads() {
    let mut vm = new_parsed_test_vm("https://page-swap.test/", "<!doctype html><body></body>");
    let result = vm
        .eval(
            r#"
        (() => {
          const event = new PageSwapEvent('swap', { bubbles: true });
          if (!(event instanceof Event) || event.type !== 'swap' || !event.bubbles
              || event.isTrusted || event.activation !== null || event.viewTransition !== null) {
            throw new Error('PageSwapEvent defaults');
          }
          for (const field of ['activation', 'viewTransition']) {
            const getter = Object.getOwnPropertyDescriptor(PageSwapEvent.prototype, field).get;
            for (const attempt of [
              () => getter.call(new Event('swap')),
              () => new PageSwapEvent('swap', { [field]: {} }),
            ]) {
              let error;
              try { attempt(); } catch (caught) { error = caught; }
              if (!(error instanceof TypeError)) throw new Error(field + ' type check');
            }
          }
          return true;
        })()
    "#,
        )
        .unwrap();
    assert_eq!(result, "true");
}
