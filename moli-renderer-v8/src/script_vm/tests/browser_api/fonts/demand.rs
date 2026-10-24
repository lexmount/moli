use super::selection_order::{order_font_data, order_vm};
use super::*;

#[test]
fn document_lifecycle_starts_used_fonts_without_geometry_or_ready_reads() {
    let mut vm = order_vm();
    vm.eval(
        r#"
        globalThis.result=[];
        globalThis.face=new FontFace('OrderFace','local("MoliDefinitelyNotAFont-0001345")');
        document.fonts.add(face);face.loaded.catch(error=>result.push(error.name));
    "#,
    )
    .unwrap();
    let publishes = vm.layout_snapshot_cache_observability_for_test().2;
    vm.prime_document_lifecycle_processing_and_record_stylesheet_network_results();
    run_font_tasks(&mut vm);
    assert_eq!(
        vm.eval("JSON.stringify(result)").unwrap(),
        r#"["NetworkError"]"#
    );
    assert_eq!(
        vm.layout_snapshot_cache_observability_for_test().2,
        publishes,
        "font discovery must not publish geometry"
    );
}

#[test]
fn unused_font_discovery_reuses_clean_inputs_and_rechecks_cssom_changes() {
    let mut vm = order_vm();
    let (_, wide) = order_font_data();
    vm.eval(&format!(r#"
        globalThis.face=new FontFace('LaterFace','url(data:font/ttf;base64,{wide})');document.fonts.add(face);
        globalThis.result=[];face.loaded.then(()=>result.push('loaded'));
        globalThis.style=document.createElement('style');document.head.append(style);
    "#)).unwrap();
    vm.prime_document_lifecycle_processing_and_record_stylesheet_network_results();
    let passes = vm.layout_pass_observability_for_test().1;
    for _ in 0..3 {
        vm.prime_document_lifecycle_processing_and_record_stylesheet_network_results();
    }
    assert_eq!(vm.layout_pass_observability_for_test().1, passes);
    assert_eq!(vm.eval("JSON.stringify(result)").unwrap(), "[]");
    vm.eval("style.sheet.insertRule('#probe {font-family:LaterFace !important}',0)")
        .unwrap();
    vm.prime_document_lifecycle_processing_and_record_stylesheet_network_results();
    run_font_tasks(&mut vm);
    assert_eq!(vm.eval("JSON.stringify(result)").unwrap(), r#"["loaded"]"#);
    assert_eq!(
        vm.eval("String(probe.getBoundingClientRect().width)")
            .unwrap(),
        "160"
    );
}

#[test]
fn rendering_loads_manual_faces_without_loading_unused_or_nonmatching_faces() {
    let mut vm = order_vm();
    let (_, wide) = order_font_data();
    vm.eval(&format!(r#"
        globalThis.result=[];
        const source='url(data:font/ttf;base64,{wide})';
        globalThis.faces=[
          new FontFace('OrderFace',source),
          new FontFace('UnusedFace',source),
          new FontFace('OrderFace',source,{{unicodeRange:'U+4E00'}}),
          new FontFace('HiddenFace',source),
          new FontFace('EmptyFace',source)
        ];
        for(const face of faces) document.fonts.add(face);
        const hidden=document.createElement('span');hidden.style.cssText='display:none;font:40px HiddenFace';hidden.textContent='AAAA';document.body.append(hidden);
        const empty=document.createElement('span');empty.style.cssText='font:40px EmptyFace';document.body.append(empty);
        faces[0].loaded.then(face=>result.push(face===faces[0]));
    "#)).unwrap();
    assert_eq!(
        vm.eval("faces.map(face=>face.status).join(',')").unwrap(),
        "unloaded,unloaded,unloaded,unloaded,unloaded"
    );
    vm.eval("document.fonts.ready.then(fonts=>result.push(fonts===document.fonts))")
        .unwrap();
    run_font_tasks(&mut vm);
    assert_eq!(
        vm.eval("String(probe.getBoundingClientRect().width)")
            .unwrap(),
        "160"
    );
    assert_eq!(
        vm.eval("faces.map(face=>face.status).join(',')").unwrap(),
        "loaded,unloaded,unloaded,unloaded,unloaded"
    );
    assert_eq!(vm.eval("JSON.stringify(result)").unwrap(), "[true,true]");
    assert_eq!(vm.document_web_font_counts_for_test().2, 1);
}

#[test]
fn rendering_rejects_missing_local_faces_and_reports_font_set_errors() {
    for observe in [false, true] {
        let mut vm = order_vm();
        vm.eval(r#"
            globalThis.result=[];globalThis.rejections=[];globalThis.errors=[];
            addEventListener('unhandledrejection',event=>{rejections.push(event.reason.name);event.preventDefault()});
            globalThis.face=new FontFace('OrderFace','local("MoliDefinitelyNotAFont-0001345")');
            document.fonts.add(face);
            document.fonts.addEventListener('loadingerror',event=>errors.push(event.fontfaces.length,event.fontfaces[0]===face));
        "#).unwrap();
        if observe {
            vm.eval("face.loaded.catch(error=>result.push(error.name))")
                .unwrap();
        }
        vm.eval("void probe.getBoundingClientRect() ").unwrap();
        run_font_tasks(&mut vm);
        assert_eq!(vm.eval("face.status").unwrap(), "error");
        assert_eq!(
            vm.eval("JSON.stringify(result)").unwrap(),
            if observe { r#"["NetworkError"]"# } else { "[]" }
        );
        assert_eq!(
            vm.eval("JSON.stringify([rejections,errors,document.fonts.status])")
                .unwrap(),
            r#"[[],[1,true],"loaded"]"#
        );
    }
}

#[test]
fn rendering_matches_capabilities_before_unicode_ranges_for_unloaded_faces() {
    let mut vm = order_vm();
    let (_, wide) = order_font_data();
    vm.eval(&format!(r#"
        globalThis.regular=new FontFace('OrderFace','url(data:font/ttf;base64,{wide})',{{weight:'400',unicodeRange:'U+4E00'}});
        globalThis.bold=new FontFace('OrderFace','url(data:font/ttf;base64,{wide})',{{weight:'700',unicodeRange:'U+0041'}});
        document.fonts.add(regular);document.fonts.add(bold);
        void probe.getBoundingClientRect();
    "#)).unwrap();
    assert_eq!(
        vm.eval("[regular.status,bold.status].join(',')").unwrap(),
        "unloaded,unloaded"
    );
    vm.eval("probe.style.fontWeight='700'").unwrap();
    refresh_layout_for_test(&mut vm);
    run_font_tasks(&mut vm);
    assert_eq!(
        vm.eval("[regular.status,bold.status].join(',')").unwrap(),
        "unloaded,loaded"
    );
    assert_eq!(
        vm.eval("String(probe.getBoundingClientRect().width)")
            .unwrap(),
        "160"
    );
}

#[test]
fn native_layout_font_demands_run_as_window_owned_tasks() {
    let mut vm = order_vm();
    let (_, wide) = order_font_data();
    vm.eval(&format!("globalThis.face=new FontFace('OrderFace','url(data:font/ttf;base64,{wide})');document.fonts.add(face)")).unwrap();
    refresh_layout_for_test(&mut vm);
    assert!(
        vm.has_ready_timeout(),
        "a native frame must schedule its demand without another JS read"
    );
    run_font_tasks(&mut vm);
    assert_eq!(
        vm.eval("[face.status,probe.getBoundingClientRect().width].join(',')")
            .unwrap(),
        "loaded,160"
    );
}

#[test]
fn font_set_ready_observes_a_native_frame_demand_before_its_task_runs() {
    let mut vm = order_vm();
    let (_, wide) = order_font_data();
    vm.eval(&format!("globalThis.face=new FontFace('OrderFace','url(data:font/ttf;base64,{wide})');document.fonts.add(face)")).unwrap();
    refresh_layout_for_test(&mut vm);
    vm.eval("globalThis.ready=false;document.fonts.ready.then(()=>ready=true)")
        .unwrap();
    assert_eq!(vm.eval("String(ready)").unwrap(), "false");
    run_font_tasks(&mut vm);
    assert_eq!(
        vm.eval("JSON.stringify([ready,face.status,probe.getBoundingClientRect().width])")
            .unwrap(),
        r#"[true,"loaded",160]"#
    );
}

#[test]
fn rendering_keeps_lower_priority_faces_unloaded_until_they_are_selected() {
    let mut vm = order_vm();
    let (narrow, wide) = order_font_data();
    vm.eval(&format!(r#"
        globalThis.pending=new FontFace('OrderFace','url(data:font/ttf;base64,{wide})');
        globalThis.loaded=new FontFace('OrderFace',Uint8Array.from(atob('{narrow}'),c=>c.charCodeAt(0)));
        document.fonts.add(pending);document.fonts.add(loaded);
        void probe.getBoundingClientRect();
    "#)).unwrap();
    assert_eq!(
        vm.eval("[pending.status,probe.getBoundingClientRect().width].join(',')")
            .unwrap(),
        "unloaded,80"
    );
    vm.eval("document.fonts.delete(pending);document.fonts.add(pending);void probe.getBoundingClientRect()").unwrap();
    assert_eq!(
        vm.eval("[pending.status,probe.getBoundingClientRect().width].join(',')")
            .unwrap(),
        "loaded,160"
    );
}

#[test]
fn geometry_from_a_promise_reaction_starts_and_settles_font_loading_in_the_checkpoint() {
    let mut vm = order_vm();
    let (_, wide) = order_font_data();
    vm.eval(&format!(r#"
        globalThis.face=new FontFace('OrderFace','url(data:font/ttf;base64,{wide})');document.fonts.add(face);
        globalThis.result=[];face.loaded.then(value=>result.push(value===face));
        Promise.resolve().then(()=>{{void probe.getBoundingClientRect()}});
    "#)).unwrap();
    assert_eq!(
        vm.eval("JSON.stringify([face.status,result])").unwrap(),
        r#"["loaded",[true]]"#
    );
}

#[test]
fn rendering_in_another_document_loads_the_shared_face_in_its_creation_window() {
    let mut vm = order_vm();
    let (_, wide) = order_font_data();
    vm.eval(&format!(r#"
        globalThis.face=new FontFace('ChildDemand','url(data:font/ttf;base64,{wide})');
        const child=document.getElementById('frame').contentDocument;
        child.body.innerHTML='<span id="childProbe" style="display:inline-block;font:40px ChildDemand">AAAA</span>';
        child.fonts.add(face);
        globalThis.result=[];face.loaded.then(value=>result.push(value===face));
        child.fonts.ready;
    "#)).unwrap();
    run_font_tasks(&mut vm);
    assert_eq!(vm.eval("JSON.stringify([face.status,result,document.fonts.has(face),document.getElementById('frame').contentDocument.getElementById('childProbe').getBoundingClientRect().width])").unwrap(), r#"["loaded",[true],false,160]"#);
}

#[tokio::test]
async fn implicit_font_network_load_waits_for_completion_and_font_set_ready() {
    let (url, request_rx, release_tx, server) = spawn_gated_font_resource_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let document_url = url.replace("/print-only.woff2", "/page");
    let (mut vm, mut completions) = new_parsed_test_vm_with_loader_and_resource_completion_queue(
        &document_url,
        "<!doctype html><span id=probe style='display:inline-block;font:40px NetworkFace'>AAAA</span>",
        &loader,
    );
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    vm.eval(&format!(r#"
        globalThis.result=[];globalThis.events=[];
        globalThis.face=new FontFace('NetworkFace','url("{url}")');document.fonts.add(face);
        document.fonts.addEventListener('loadingdone',event=>events.push(event.fontfaces.length,event.fontfaces[0]===face));
        face.loaded.then(value=>result.push(value===face));
        document.fonts.ready.then(value=>result.push(value===document.fonts));
    "#)).unwrap();
    let request = tokio::time::timeout(Duration::from_secs(2), request_rx)
        .await
        .unwrap()
        .unwrap();
    assert!(request.contains("GET /print-only.woff2 "), "{request}");
    assert_eq!(
        vm.eval("JSON.stringify([face.status,document.fonts.status,result])")
            .unwrap(),
        r#"["loading","loading",[]]"#
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_subresource_request_count(),
        1
    );
    release_tx.send(()).unwrap();
    server.await.unwrap();
    assert!(
        tokio::time::timeout(
            Duration::from_secs(2),
            completions.wait_for_arrival_without_timeout()
        )
        .await
        .unwrap()
    );
    let completion = completions.pop_next_async_subresource_event().unwrap();
    let activity = vm
        .complete_async_subresource_fetch_event_body(completion)
        .unwrap();
    vm.finish_async_subresource_body_checkpoint_for_test(activity)
        .unwrap();
    run_font_tasks(&mut vm);
    assert_eq!(vm.eval("JSON.stringify([face.status,result,events,document.fonts.status,probe.getBoundingClientRect().width])").unwrap(), r#"["loaded",[true,true],[1,true],"loaded",96]"#);
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_subresource_request_count(),
        0
    );
}
