use super::*;

#[tokio::test]
async fn freshly_connected_inline_transition_establishes_its_first_style() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://css-lifecycle.test/inline-first-connection",
        "<html><head></head><body></body></html>",
        &loader,
    );
    vm.eval(r#"
      globalThis.events=[];
      globalThis.box=document.createElement('div');
      box.style.cssText='opacity:0;transition:opacity .04s';
      for(const type of ['transitionrun','transitionstart','transitionend'])box.addEventListener(type,e=>events.push(e.type));
    "#).unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
        .await
        .unwrap();
    vm.eval("document.body.appendChild(box)").unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
        .await
        .unwrap();
    assert_eq!(vm.eval("events.length").unwrap(), "0");
    vm.eval("box.style.opacity='1'").unwrap();
    wait_for_css_events(&mut vm, &loader, "events.length.toString()", "3").await;
    assert_eq!(
        vm.eval("events.join('|')").unwrap(),
        "transitionrun|transitionstart|transitionend"
    );
}

#[tokio::test]
async fn freshly_connected_elements_start_stylesheet_animations_without_prior_state() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for property in ["animation", r"\61nimation"] {
        let mut vm = new_parsed_page_task_executor_test_vm(
            "https://css-lifecycle.test/first-connection",
            &format!(
                r#"<html><head><style>
          @keyframes inserted {{from {{left:0px}} to {{left:10px}}}}
          #fresh {{position:relative;{property}:inserted .04s linear}}
        </style></head><body></body></html>"#
            ),
            &loader,
        );
        vm.eval("globalThis.events=[];for(const type of ['animationstart','animationend'])document.addEventListener(type,e=>events.push(e.type))").unwrap();
        vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
            .await
            .unwrap();
        vm.eval(
            "const box=document.createElement('div');box.id='fresh';document.body.appendChild(box)",
        )
        .unwrap();
        wait_for_css_events(&mut vm, &loader, "events.length.toString()", "2").await;
        assert_eq!(
            vm.eval("events.join('|')").unwrap(),
            "animationstart|animationend"
        );
    }
}

#[tokio::test]
async fn focus_and_blur_schedule_transition_lifecycles_without_an_attribute_change() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://css-lifecycle.test/focus-transition",
        r#"<html><head><style>
          #box {width:50px;transition:width .04s}
          #box:focus {width:200px}
        </style></head><body><div id=box tabindex=0></div></body></html>"#,
        &loader,
    );
    vm.eval("globalThis.events=[];globalThis.box=document.getElementById('box');for(const type of ['transitionrun','transitionstart','transitionend','transitioncancel'])box['on'+type]=e=>events.push(e.type);getComputedStyle(box).width").unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
        .await
        .unwrap();
    vm.eval("box.focus()").unwrap();
    assert_eq!(vm.eval("document.activeElement===box").unwrap(), "true");
    wait_for_css_events(
        &mut vm,
        &loader,
        "events.filter(e=>e==='transitionend').length.toString()",
        "1",
    )
    .await;
    vm.eval("box.blur()").unwrap();
    wait_for_css_events(
        &mut vm,
        &loader,
        "events.filter(e=>e==='transitionend').length.toString()",
        "2",
    )
    .await;
    assert_eq!(
        vm.eval("events.join('|')").unwrap(),
        "transitionrun|transitionstart|transitionend|transitionrun|transitionstart|transitionend"
    );
}
