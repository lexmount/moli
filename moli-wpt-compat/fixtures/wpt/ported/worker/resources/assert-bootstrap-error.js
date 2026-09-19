function assert_worker_bootstrap_error(event, worker) {
  assert_equals(Object.getPrototypeOf(event), Event.prototype,
    "bootstrap failure uses Event, not ErrorEvent");
  assert_equals(event.type, "error");
  assert_equals(event.target, worker);
  assert_true(event.isTrusted);
  assert_false(event.bubbles);
  assert_false(event.cancelable);
  assert_false(event.composed);
  assert_false(event.defaultPrevented);
  event.preventDefault();
  assert_false(event.defaultPrevented, "bootstrap errors cannot be canceled");
  for (const name of ["message", "filename", "lineno", "colno", "error"]) {
    assert_false(name in event, "bootstrap Event has no ErrorEvent." + name);
  }
}
