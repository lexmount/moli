use super::*;

#[tokio::test(flavor = "current_thread")]
async fn resize_observer_delivery_ignores_inherited_array_index_accessors() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm("https://resize-array-prototype.test/");
    vm.eval(
        r#"
const target = document.body.appendChild(document.createElement('div'));
target.style.cssText = 'width:10px;height:20px';
globalThis.log = '';
let traps = 0;
const observer = new ResizeObserver(entries => {
  const entry = entries[0];
  log = entries.length + ':' + (entry.target === target) + ':' + entry.contentRect.width + ':' +
    entry.contentBoxSize[0].blockSize + ':' + entry.borderBoxSize[0].inlineSize + ':' + traps;
  observer.disconnect();
});
observer.observe(target);
Object.defineProperty(Array.prototype, '0', {
  configurable: true,
  get() { traps++; return undefined; },
  set() { traps++; }
});
// Avoid serializing Array.prototype as the evaluation result in the test host.
void 0;
"#,
    )
    .unwrap();
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .unwrap();
    assert_eq!(
        vm.eval("delete Array.prototype[0]; log").unwrap(),
        "1:true:10:20:10:0"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn rendering_observers_follow_animation_callbacks_and_precede_focus_fixup() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm("https://rendering-observers.test/");
    vm.eval(r#"
const target = document.body.appendChild(document.createElement('button'));
target.style.cssText = 'width:40px;height:30px;padding:0;border:0';
target.focus();
globalThis.log = [];
target.addEventListener('blur', () => log.push('blur'));
const resize = new ResizeObserver(entries => {
  log.push('resize:' + entries[0].contentRect.width + ':' + (document.activeElement === target));
  resize.disconnect();
  queueMicrotask(() => log.push('resize-microtask'));
});
const intersection = new IntersectionObserver(() => {
  log.push('intersection:' + (document.activeElement === document.body));
  intersection.disconnect();
});
intersection.observe(target);
resize.observe(target);
queueMicrotask(() => log.push('microtask'));
requestAnimationFrame(() => {
  log.push('frame');
  target.style.width = '90px';
  target.disabled = true;
});
for (const name of ['requestAnimationFrame', 'setTimeout', 'setInterval']) {
  Object.defineProperty(window, name, {configurable: true, get() { throw new Error('author ' + name); }});
}
"#).unwrap();
    assert_eq!(vm.eval("log.join('|')").unwrap(), "microtask");
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .unwrap();
    assert_eq!(
        vm.eval("log.join('|')").unwrap(),
        "microtask|frame|resize:90:true|resize-microtask|blur|intersection:true"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn resize_observer_depth_loop_refreshes_layout_and_defers_skipped_targets() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm("https://resize-depth-loop.test/");
    vm.eval(r#"
const shallow = document.body.appendChild(document.createElement('div'));
const middle = shallow.appendChild(document.createElement('div'));
const deep = middle.appendChild(document.createElement('div'));
const targets = [shallow, middle, deep];
targets.forEach((target, i) => { target.id = String(i); target.style.cssText = 'width:10px;height:10px'; });
globalThis.log = [];
let frame = 0;
requestAnimationFrame(() => { frame = 1; requestAnimationFrame(() => { frame = 2; }); });
addEventListener('error', event => {
  if (event.message === 'ResizeObserver loop completed with undelivered notifications.') {
    log.push('error:' + frame);
    event.preventDefault();
  }
});
let calls = 0;
const observer = new ResizeObserver(entries => {
  calls++;
  log.push(frame + ':' + entries.map(entry => entry.target.id + '=' + entry.contentRect.width).join(','));
  queueMicrotask(() => log.push('microtask:' + calls));
  if (calls < 4) targets.forEach(target => { target.style.width = (10 + calls) + 'px'; });
  else observer.disconnect();
});
targets.forEach(target => observer.observe(target));
"#).unwrap();
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .unwrap();
    assert_eq!(
        vm.eval("log.join('|')").unwrap(),
        "1:0=10,1=10,2=10|microtask:1|1:1=11,2=11|microtask:2|1:2=12|microtask:3|error:1|2:0=13,1=13,2=13|microtask:4"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn resize_observer_uses_constructor_order_and_disconnect_cancels_active_delivery() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm("https://resize-registration-order.test/");
    vm.eval(r#"
const target = document.body.appendChild(document.createElement('div'));
target.style.cssText = 'width:10px;height:10px';
globalThis.log = [];
const first = new ResizeObserver(() => { log.push('first'); last.disconnect(); first.disconnect(); });
const second = new ResizeObserver(() => { log.push('second'); second.disconnect(); });
const last = new ResizeObserver(() => log.push('last'));
last.observe(target);
second.observe(target);
first.observe(target);
first.disconnect();
first.observe(target);
"#).unwrap();
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .unwrap();
    assert_eq!(vm.eval("log.join('|')").unwrap(), "first|second");
}

#[tokio::test(flavor = "current_thread")]
async fn resize_observer_samples_flattened_shadow_depth_for_reentrant_observers() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm("https://resize-shadow-depth.test/");
    vm.eval(r#"
const host = document.body.appendChild(document.createElement('div'));
host.style.cssText = 'width:10px;height:10px';
globalThis.log = [];
let frame = 0;
requestAnimationFrame(() => { frame = 1; requestAnimationFrame(() => { frame = 2; }); });
addEventListener('error', event => { log.push('error'); event.preventDefault(); });
const first = new ResizeObserver(() => {
  log.push('host:' + frame);
  first.disconnect();
  const root = host.attachShadow({mode:'open'});
  const child = root.appendChild(document.createElement('div'));
  child.style.cssText = 'width:5px;height:5px';
  const second = new ResizeObserver(entries => { log.push('child:' + frame + ':' + entries[0].contentRect.width); second.disconnect(); });
  second.observe(child);
});
first.observe(host);
"#).unwrap();
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .unwrap();
    assert_eq!(vm.eval("log.join('|')").unwrap(), "host:1|child:1:5");
}

#[tokio::test(flavor = "current_thread")]
async fn rendering_observers_do_not_continue_a_replaced_document_or_retired_callback() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm("https://resize-exact-document.test/");
    vm.eval(
        r#"
const target = document.body.appendChild(document.createElement('div'));
globalThis.log = [];
let phase = 'old';
const first = new ResizeObserver(() => {
  log.push('first');
  first.disconnect();
  document.open();
  document.write('<!doctype html><button id="new">new</button>');
  document.close();
  const replacement = document.getElementById('new');
  replacement.focus();
  replacement.disabled = true;
  requestAnimationFrame(() => { phase = 'replacement'; log.push('replacement-frame:' + document.activeElement.id); });
});
const second = new ResizeObserver(() => { log.push('second:' + phase); second.disconnect(); });
first.observe(target); second.observe(target);
"#,
    )
    .unwrap();
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .unwrap();
    assert_eq!(
        vm.eval("log.join('|')").unwrap(),
        "first|replacement-frame:new|second:replacement"
    );
    vm.eval(
        r#"
const iframe = document.body.appendChild(document.createElement('iframe'));
const child = iframe.contentWindow;
child.document.body.innerHTML = '<div id="target" style="width:10px;height:10px"></div>';
const observer = new ResizeObserver(child.Function("parent.log.push('retired')"));
observer.observe(child.document.getElementById('target'));
iframe.remove();
"#,
    )
    .unwrap();
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .unwrap();
    assert_eq!(vm.eval("log.includes('retired')").unwrap(), "false");
}

#[tokio::test(flavor = "current_thread")]
async fn rendering_observer_watchdog_bounds_callbacks_and_microtasks() {
    use crate::v8_execution_watchdog::{V8ExecutionWatchdog, V8ExecutionWatchdogKind};
    let _budget = V8ExecutionWatchdog::override_timeout_for_test(
        V8ExecutionWatchdogKind::RenderingObservers,
        std::time::Duration::from_millis(100),
    );
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for observer in ["ResizeObserver", "IntersectionObserver"] {
        for runaway in [
            "while (true) {}",
            "queueMicrotask(() => { while (true) {} });",
        ] {
            let mut vm =
                new_storage_page_task_executor_test_vm("https://rendering-observer-watchdog.test/");
            vm.eval(&format!(
                r#"
const target = document.body.appendChild(document.createElement('div'));
globalThis.calls = 0;
const observer = new {observer}(() => {{ calls++; observer.disconnect(); {runaway} }});
observer.observe(target);
"#
            ))
            .unwrap();
            vm.advance_timers_until_deadline_for_test(&loader)
                .await
                .unwrap();
            assert_eq!(
                vm.eval("[calls, 6 * 7].join('|')").unwrap(),
                "1|42",
                "{observer}: {runaway}"
            );
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn rendering_observers_wake_for_cssom_and_adopted_sheet_changes() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm("https://resize-cssom.test/");
    vm.eval(
        r#"
const style = document.head.appendChild(document.createElement('style'));
style.textContent = '.probe { width:10px;height:10px }';
const target = document.body.appendChild(document.createElement('div'));
target.className = 'probe';
globalThis.log = [];
const observer = new ResizeObserver(entries => log.push(entries[0].contentRect.width));
observer.observe(target);
"#,
    )
    .unwrap();
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .unwrap();
    assert_eq!(vm.eval("log.join('|')").unwrap(), "10");
    for (script, expected) in [
        ("style.sheet.cssRules[0].style.width = '20px'", "10|20"),
        (
            "style.sheet.deleteRule(0); style.sheet.insertRule('.probe { width:30px;height:10px }')",
            "10|20|30",
        ),
        (
            "globalThis.sheet = new CSSStyleSheet(); sheet.replaceSync('.probe { width:40px;height:10px }'); document.adoptedStyleSheets = [sheet]",
            "10|20|30|40",
        ),
        (
            "sheet.replaceSync('.probe { width:50px;height:10px }')",
            "10|20|30|40|50",
        ),
    ] {
        vm.eval(script).unwrap();
        vm.advance_timers_until_deadline_for_test(&loader)
            .await
            .unwrap();
        assert_eq!(vm.eval("log.join('|')").unwrap(), expected, "{script}");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn resize_observer_tracks_cross_document_targets_without_relying_on_the_mutator_realm() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm("https://resize-cross-document.test/");
    vm.eval(
        r#"
const iframe = document.body.appendChild(document.createElement('iframe'));
const child = iframe.contentWindow;
child.document.body.innerHTML = '<div id="target" style="width:10px;height:10px"></div>';
const target = child.document.getElementById('target');
globalThis.log = [];
const observer = new ResizeObserver(entries => log.push(entries[0].contentRect.width));
observer.observe(target);
"#,
    )
    .unwrap();
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .unwrap();
    vm.eval("child.eval(\"document.getElementById('target').style.width = '20px'\")")
        .unwrap();
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .unwrap();
    assert_eq!(vm.eval("log.join('|')").unwrap(), "10|20");
    vm.eval("ResizeObserver.prototype.disconnect.call(observer); target.style.width = '30px'")
        .unwrap();
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .unwrap();
    assert_eq!(vm.eval("log.join('|')").unwrap(), "10|20");
}

#[tokio::test(flavor = "current_thread")]
async fn resize_observer_boxes_use_frozen_padding_axes_and_zoom_and_ignore_inline_fragments() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm("https://resize-box-geometry.test/");
    vm.eval(r#"
document.body.innerHTML = '<div id="block"></div><div id="vertical"></div><span id="inline">text</span><span id="inline-block">text</span>';
for (const target of document.body.children) target.style.cssText = 'width:41px;height:23px;padding:2px 3px;border:1px solid';
vertical.style.writingMode = 'vertical-rl';
document.getElementById('inline-block').style.display = 'inline-block';
document.body.style.zoom = 2;
globalThis.log = [];
const observer = new ResizeObserver(entries => {
  log = entries.map(entry => [entry.target.id, entry.contentRect.x, entry.contentRect.y, entry.contentRect.width, entry.contentRect.height,
    entry.contentBoxSize[0].inlineSize, entry.contentBoxSize[0].blockSize,
    entry.borderBoxSize[0].inlineSize, entry.borderBoxSize[0].blockSize,
    entry.devicePixelContentBoxSize[0].inlineSize, entry.devicePixelContentBoxSize[0].blockSize].join(':'));
  observer.disconnect();
});
for (const target of document.body.children) observer.observe(target, {box:'device-pixel-content-box'});
"#).unwrap();
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .unwrap();
    assert_eq!(
        vm.eval("log.join('|')").unwrap(),
        "block:3:2:41:23:41:23:49:29:82:46|vertical:3:2:41:23:23:41:29:49:46:82|inline:0:0:0:0:0:0:0:0:0:0|inline-block:3:2:41:23:41:23:49:29:82:46"
    );
}
