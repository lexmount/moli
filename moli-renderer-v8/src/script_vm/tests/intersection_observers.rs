use super::*;

#[tokio::test]
async fn intersection_observer_callbacks_can_drain_later_observers() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm("https://intersection-queues.test/");
    vm.eval(
        r#"
const target = name => {
  const el = document.body.appendChild(document.createElement('div'));
  el.id = name; el.style.cssText = 'width:20px;height:20px';
  return el;
};
const a = target('a'), b = target('b');
globalThis.queueOrder = [];
const first = new IntersectionObserver(() => {
  queueOrder.push('first');
  queueOrder.push('drained:' + second.takeRecords().map(entry => entry.target.id).join(','));
  first.disconnect(); second.disconnect();
});
const second = new IntersectionObserver(() => queueOrder.push('unexpected'));
const third = new IntersectionObserver(entries => {
  queueOrder.push('third:' + entries.map(entry => entry.target.id).join(','));
  third.disconnect();
});
first.observe(a);
second.observe(b); second.observe(a);
third.observe(a); third.observe(b);
"#,
    )
    .unwrap();
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .unwrap();
    assert_eq!(
        vm.eval("queueOrder.join('|')").unwrap(),
        "first|drained:b,a|third:a,b"
    );
}
