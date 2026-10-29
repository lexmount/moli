use super::*;

#[tokio::test]
async fn removing_or_hiding_a_running_effect_cancels_it_and_reveal_restarts_animation() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for mutation in [
        "box.remove()",
        "box.style.display='none'",
        "wrapper.style.display='none'",
    ] {
        let mut vm = new_parsed_page_task_executor_test_vm(
            "https://css-lifecycle.test/rendered-lifetime",
            r#"<html><head><style>
              @keyframes move {from {left:0px} to {left:10px}}
              #box {position:relative;opacity:0;transition:opacity 60s}
              #box.active {opacity:1;animation:move 60s linear}
            </style></head><body><div id=parent><div id=box></div></div></body></html>"#,
            &loader,
        );
        vm.eval(r#"
          globalThis.events=[];globalThis.box=document.getElementById('box');
          globalThis.wrapper=document.getElementById('parent');
          for(const type of ['animationstart','animationend','animationcancel','transitionrun','transitionstart','transitionend','transitioncancel']) {
            box.addEventListener(type,e=>events.push([e.type,e.elapsedTime]));
          }
        "#).unwrap();
        vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
            .await
            .unwrap();
        vm.eval("box.className='active'").unwrap();
        vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
            .await
            .unwrap();
        vm.eval(mutation).unwrap();
        vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
            .await
            .unwrap();
        assert_eq!(
            vm.eval("events.filter(e=>e[0].endsWith('cancel')).map(e=>e[0]).sort().join('|')")
                .unwrap(),
            "animationcancel|transitioncancel",
            "{mutation}"
        );
        vm.eval("document.getElementById('parent').appendChild(box);box.style.display='';wrapper.style.display=''").unwrap();
        vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
            .await
            .unwrap();
        assert_eq!(
            vm.eval("events.filter(e=>e[0]==='animationstart').length")
                .unwrap(),
            "2",
            "{mutation}"
        );
        assert_eq!(
            vm.eval("events.filter(e=>e[0]==='transitionrun').length")
                .unwrap(),
            "1",
            "revealing must not transition from hidden style"
        );
        assert_eq!(
            vm.eval("events.filter(e=>e[0].endsWith('end')).length")
                .unwrap(),
            "0"
        );
    }
}

#[tokio::test]
async fn initially_hidden_elements_and_hidden_style_observations_do_not_create_transitions() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for hidden in ["#box", "#parent"] {
        let mut vm = new_parsed_page_task_executor_test_vm(
            "https://css-lifecycle.test/hidden-style-observation",
            &format!(
                r#"<html><head><style>
              {hidden} {{display:none}}
              #box {{opacity:0;transition:opacity .04s}}
              #box.active {{opacity:1}}
            </style></head><body><div id=parent><div id=box></div></div></body></html>"#
            ),
            &loader,
        );
        vm.eval(r#"
          globalThis.events=[];globalThis.box=document.getElementById('box');
          for (const type of ['transitionrun','transitionstart','transitionend','transitioncancel']) box.addEventListener(type,e=>events.push(e.type));
          getComputedStyle(box).opacity;
        "#).unwrap();
        vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
            .await
            .unwrap();
        vm.eval("box.className='active';getComputedStyle(box).opacity")
            .unwrap();
        vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
            .await
            .unwrap();
        assert_eq!(vm.eval("events.length").unwrap(), "0", "{hidden}");
        vm.eval(
            "box.style.display='block';document.getElementById('parent').style.display='block'",
        )
        .unwrap();
        vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
            .await
            .unwrap();
        assert_eq!(
            vm.eval("events.length").unwrap(),
            "0",
            "{hidden}: first appearance has no before-change style"
        );
    }
}
