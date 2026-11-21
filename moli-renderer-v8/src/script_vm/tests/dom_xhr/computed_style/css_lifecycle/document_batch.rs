use super::*;

#[tokio::test]
async fn completed_transition_batches_are_independent_of_dom_callback_order() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for order in [
        "<div id=a></div><div id=b></div>",
        "<div id=b></div><div id=a></div>",
    ] {
        let mut vm = new_parsed_page_task_executor_test_vm(
            "https://css-lifecycle.test/document-batch",
            &format!(
                r#"<html><head><style>
              #a, #b {{ opacity: 0; transition: opacity 0.04s; }}
              .active #a, .active #b {{ opacity: 1; }}
            </style></head><body>{order}</body></html>"#
            ),
            &loader,
        );
        vm.eval(r#"
            globalThis.events=[];
            globalThis.a=document.getElementById('a');
            globalThis.b=document.getElementById('b');
            for (const type of ['transitionrun','transitionstart','transitionend','transitioncancel']) {
              b.addEventListener(type, e=>events.push(e.type));
            }
            a.addEventListener('transitionend', ()=>{b.style.opacity='0.5'}, {once:true});
        "#).unwrap();
        vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
            .await
            .unwrap();
        vm.eval("document.body.className='active'").unwrap();
        wait_for_css_events(
            &mut vm,
            &loader,
            "events.filter(e=>e==='transitionend').length.toString()",
            "2",
        )
        .await;
        assert_eq!(
            vm.eval("events.join('|')").unwrap(),
            "transitionrun|transitionstart|transitionend|transitionrun|transitionstart|transitionend",
            "DOM order: {order}"
        );
    }
}

#[tokio::test]
async fn adopting_a_later_element_in_a_callback_does_not_start_it_in_the_old_document() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://css-lifecycle.test/adopt-batch",
        "<html><head></head><body><div id=a></div><div id=b></div><iframe id=child></iframe></body></html>",
        &loader,
    );
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 64)
        .await
        .unwrap();
    vm.eval(r#"
        globalThis.events=[];
        globalThis.a=document.getElementById('a');
        globalThis.b=document.getElementById('b');
        globalThis.child=document.getElementById('child').contentDocument;
        const css='@keyframes move {from {left:0px} to {left:10px}} .moving {position:relative;animation:move .04s linear}';
        for (const doc of [document,child]) doc.head.appendChild(doc.createElement('style')).textContent=css;
        for (const type of ['animationstart','animationend','animationcancel']) b.addEventListener(type,e=>events.push(e.type));
        a.addEventListener('animationstart', ()=>{
          child.body.appendChild(child.adoptNode(b));b.className='moving';
        }, {once:true});
    "#).unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 64)
        .await
        .unwrap();
    vm.eval("a.className='moving'").unwrap();
    wait_for_css_events(
        &mut vm,
        &loader,
        "events.filter(e=>e==='animationend').length.toString()",
        "1",
    )
    .await;
    assert_eq!(
        vm.eval("events.join('|')").unwrap(),
        "animationstart|animationend"
    );
}
