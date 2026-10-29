use super::*;

#[tokio::test(flavor = "current_thread")]
async fn visual_viewport_borrowed_getters_use_the_receiver_window() {
    for policy in [
        moli_page_types::LayoutPolicy::Mock,
        moli_page_types::LayoutPolicy::OnDemand,
    ] {
        let mut vm = new_storage_html_test_vm("https://visual-viewport-receiver.test/");
        vm.set_layout_policy(policy);
        vm.eval(
            r#"
            globalThis.frame=document.createElement('iframe');
            frame.style.cssText='width:123px;height:87px;border:0';
            frame.srcdoc='<body>child</body>';document.body.append(frame);
            'ready'
        "#,
        )
        .unwrap();
        run_child_navigation_commit_and_host_load_for_test(&mut vm, "viewport child setup").await;
        if policy == moli_page_types::LayoutPolicy::OnDemand {
            vm.publish_layout_for_test().unwrap();
        }
        let builds = vm.layout_pass_observability_for_test().1;
        let facts = vm.eval(r#"JSON.stringify((()=>{
            const parentViewport=window.visualViewport;
            const childViewport=frame.contentWindow.visualViewport;
            const getter=(viewport,name)=>Object.getOwnPropertyDescriptor(Object.getPrototypeOf(viewport),name).get;
            globalThis.savedViewport=childViewport;
            globalThis.childWidth=getter(childViewport,'width');
            globalThis.childHeight=getter(childViewport,'height');
            const rows=['width','height'].map(name=>{
                const parentGetter=getter(parentViewport,name);
                const childGetter=getter(childViewport,name);
                return [parentGetter.call(childViewport),childViewport[name],
                    childGetter.call(parentViewport),parentViewport[name]];
            });
            return {distinct:parentViewport.width!==childViewport.width,
                rows,brands:['width','height'].map(name=>{
                    try{getter(parentViewport,name).call({});return 'accepted'}catch(e){return e.name}
                })};
        })())"#).unwrap();
        let facts: serde_json::Value = serde_json::from_str(&facts).unwrap();
        assert_eq!(
            facts["distinct"], true,
            "fixture must have distinct viewports"
        );
        assert_eq!(
            facts["brands"],
            serde_json::json!(["TypeError", "TypeError"])
        );
        for row in facts["rows"].as_array().unwrap() {
            assert_eq!(row[0], row[1], "parent getter on child: {policy:?}");
            assert_eq!(row[2], row[3], "child getter on parent: {policy:?}");
        }
        assert_eq!(vm.layout_pass_observability_for_test().1, builds);
        vm.eval("frame.remove(); 'removed'").unwrap();
        assert_eq!(
            vm.eval("JSON.stringify([savedViewport.width,savedViewport.height,childWidth.call(savedViewport),childHeight.call(savedViewport)])").unwrap(),
            "[0,0,0,0]",
            "a retired child viewport must not borrow the parent dimensions: {policy:?}"
        );
        assert_eq!(vm.layout_pass_observability_for_test().1, builds);
    }
}
