use super::*;

#[test]
fn bar_prop_identity_brand_and_replaceable_surface() {
    let mut vm = new_storage_test_vm("https://barprop.test/");
    assert_eq!(
        vm.lazy_constructor_materialization_count_for_test("BarProp")
            .unwrap(),
        0
    );
    let result = vm.eval(r#"
(() => {
  const names = ['locationbar', 'menubar', 'personalbar', 'scrollbars', 'statusbar', 'toolbar'];
  Array.prototype[0] = {visible: 'poisoned'};
  const bars = names.map(name => window[name]);
  delete Array.prototype[0];
  const result = [new Set(bars).size === 6,
    bars.every(bar => bar.visible && bar instanceof BarProp),
    bars.every(bar => Object.prototype.toString.call(bar) === '[object BarProp]')];
  const descriptor = Object.getOwnPropertyDescriptor(BarProp.prototype, 'visible');
  result.push(descriptor.enumerable && descriptor.configurable && descriptor.set === undefined,
    descriptor.get.name === 'get visible' && descriptor.get.length === 0);
  try { new BarProp(); result.push(false); } catch (e) { result.push(e instanceof TypeError); }
  const frame = document.createElement('iframe');
  (document.body || document.documentElement || document).appendChild(frame);
  const other = frame.contentWindow;
  const original = other.BarProp;
  other.BarProp = function Poisoned() { throw new Error('author constructor'); };
  const bar = other.locationbar;
  const getter = Object.getOwnPropertyDescriptor(original.prototype, 'visible').get;
  result.push(bar instanceof original, getter.call(locationbar) === true,
    descriptor.get.call(bar) === true,
    Object.getOwnPropertyDescriptor(window, 'locationbar').get.call(other) === bar);
  let traps = 0;
  const revoked = Proxy.revocable(bar, {}); revoked.revoke();
  for (const fake of [{}, Object.create(bar), new Proxy(bar, {get() {traps++;}}), revoked.proxy]) {
    try { getter.call(fake); result.push(false); }
    catch (e) { result.push(e instanceof other.TypeError && !(e instanceof TypeError)); }
  }
  result.push(traps === 0);
  const windowGetter = Object.getOwnPropertyDescriptor(other, 'menubar').get;
  try { windowGetter.call({}); result.push(false); }
  catch (e) { result.push(e instanceof other.TypeError); }
  for (const name of names) {
    const d = Object.getOwnPropertyDescriptor(window, name);
    const value = {toString() {throw new Error('unexpected conversion');}};
    d.set.call(window, value);
    const replaced = Object.getOwnPropertyDescriptor(window, name);
    result.push(replaced.value === value && replaced.writable && replaced.enumerable && replaced.configurable,
      d.get.call(window) === bars[names.indexOf(name)]);
  }
  frame.remove();
  result.push(getter.call(bar) === false);
  return result.every(Boolean) ? 'ok' : JSON.stringify(result);
})()
"#).unwrap();
    assert_eq!(result, "ok");
}

#[test]
fn bar_props_follow_child_window_lifetime() {
    let mut vm = new_storage_test_vm("https://barprop.test/child");
    let result = vm
        .eval(
            r#"
(() => {
  const names = ['locationbar', 'menubar', 'personalbar', 'scrollbars', 'statusbar', 'toolbar'];
  const frame = document.createElement('iframe');
  const parent = document.body || document.documentElement || document;
  parent.appendChild(frame);
  const window = frame.contentWindow;
  const bars = names.map(name => window[name]);
  bars[0].marker = 42;
  const prototype = Object.getPrototypeOf(bars[0]);
  window.document.open(); window.document.write('<p>stream'); window.document.close();
  const results = [names.every((name, index) => window[name] === bars[index]),
    bars[0].marker === 42, Object.getPrototypeOf(bars[0]) === prototype,
    bars.every(bar => bar.visible)];
  frame.remove();
  results.push(names.every((name, index) => window[name] === bars[index]),
    bars.every(bar => !bar.visible));
  parent.appendChild(frame);
  results.push(frame.contentWindow.locationbar !== bars[0],
    frame.contentWindow.locationbar.visible, bars.every(bar => !bar.visible));
  frame.remove();
  const untouched = document.createElement('iframe');
  parent.appendChild(untouched);
  const retired = untouched.contentWindow;
  untouched.remove();
  results.push(retired.locationbar.visible === false, retired.locationbar === retired.locationbar);
  return results.every(Boolean) ? 'ok' : JSON.stringify(results);
})()
"#,
        )
        .unwrap();
    assert_eq!(result, "ok");
}

#[tokio::test]
async fn bar_props_follow_popup_features_and_close() {
    let loader = static_http_loader([]);
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("https://barprop.test/popup", &loader);
    let result = vm.eval(r#"
(() => {
  const names = ['locationbar', 'menubar', 'personalbar', 'scrollbars', 'statusbar', 'toolbar'];
  const results = [];
  for (const [features, visible] of [['', true], ['popup', false], ['popup=0', true],
      ['width=300', false], ['location,menubar,scrollbars,status', true], ['popup=0,width=300', true]]) {
    const popup = window.open('about:blank', '', features);
    const bars = names.map(name => popup[name]);
    results.push(new Set(bars).size === 6, bars.every(bar => bar.visible === visible));
    const frame = popup.document.createElement('iframe'); popup.document.body.append(frame);
    const nested = frame.contentDocument.createElement('iframe'); frame.contentDocument.body.append(nested);
    results.push(names.every(name => frame.contentWindow[name].visible === visible),
      names.every(name => nested.contentWindow[name].visible === visible));
    popup.document.open(); popup.document.write('<p>stream'); popup.document.close();
    results.push(names.every((name, index) => popup[name] === bars[index]),
      bars.every(bar => bar.visible === visible));
    popup.close();
    results.push(bars.every(bar => bar.visible === visible));
  }
  globalThis.barPopup = window.open('about:blank');
  globalThis.closingBar = barPopup.locationbar;
  globalThis.barPagehide = 'pending';
  barPopup.onpagehide = () => { barPagehide = closingBar.visible; };
  barPopup.close();
  results.push(closingBar.visible);
  return results.every(Boolean) ? 'ok' : JSON.stringify(results);
})()
"#).unwrap();
    assert_eq!(result, "ok");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(barPagehide === true && closingBar.visible === false)",
        "true",
        "popup BarProp close",
    )
    .await;
}
