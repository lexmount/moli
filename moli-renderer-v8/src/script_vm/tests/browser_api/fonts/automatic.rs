use super::selection_order::{order_font_data, order_vm};
use super::*;
use crate::protocol_types::OptionalResourceFetchMask;

fn automatic_vm() -> StandaloneScriptVmHarness {
    let vm = order_vm();
    vm.current_main_document_resource_loader()
        .unwrap()
        .request_client()
        .set_optional_resource_fetch_mask(OptionalResourceFetchMask::FONT);
    vm
}

#[test]
fn automatic_font_requests_follow_the_document_resource_policy() {
    let (_, wide) = order_font_data();
    for mask in [
        OptionalResourceFetchMask::NONE,
        OptionalResourceFetchMask::IMAGE,
        OptionalResourceFetchMask::FONT,
        OptionalResourceFetchMask::ALL,
    ] {
        for child in [false, true] {
            let mut vm = order_vm();
            vm.set_layout_policy(moli_page_types::LayoutPolicy::Mock);
            vm.current_main_document_resource_loader()
                .unwrap()
                .request_client()
                .set_optional_resource_fetch_mask(mask);
            let document = if child {
                "document.getElementById('frame').contentDocument"
            } else {
                "document"
            };
            vm.eval(&format!(r#"
                globalThis.targetDocument={document};
                globalThis.face=new FontFace('PolicyFace','url(data:font/ttf;base64,{wide})');
                targetDocument.fonts.add(face);
                const style=targetDocument.createElement('style');
                style.textContent='@font-face{{font-family:CssPolicyFace;src:url(data:font/ttf;base64,{wide})}}';
                targetDocument.head.append(style);
                targetDocument.body.innerHTML='<span style="font-family:PolicyFace">AAAA</span><span style="font-family:CssPolicyFace">AAAA</span>';
                globalThis.cssFace=[...targetDocument.fonts].find(face=>face.family==='CssPolicyFace');
                globalThis.ready=[];
            "#)).unwrap();
            vm.prime_document_lifecycle_processing_and_record_stylesheet_network_results();
            vm.eval("targetDocument.fonts.ready.then(()=>ready.push(true));void targetDocument.fonts.status")
                .unwrap();
            run_font_tasks(&mut vm);
            let enabled = mask.contains(OptionalResourceFetchMask::FONT);
            assert_eq!(
                vm.eval("[face.status,cssFace.status].join(',')").unwrap(),
                if enabled {
                    "loaded,loaded"
                } else {
                    "unloaded,unloaded"
                },
                "mask={mask:?}, child={child}"
            );
            assert_eq!(vm.eval("JSON.stringify(ready)").unwrap(), "[true]");
            if !enabled {
                assert_eq!(discovery_count(&mut vm), 0);
            }
            assert_eq!(vm.layout_pass_observability_for_test().1, 0);
        }
    }
}

#[test]
fn explicit_font_loading_keeps_working_without_automatic_font_requests() {
    let mut vm = order_vm();
    let (_, wide) = order_font_data();
    vm.eval(&format!(r#"
        globalThis.face=new FontFace('OrderFace','url(data:font/ttf;base64,{wide})');
        document.fonts.add(face);
        globalThis.results=[];
        face.load().then(value=>results.push(value===face));
        document.fonts.load('40px OrderFace','AAAA').then(faces=>results.push(faces.length===1 && faces[0]===face));
    "#)).unwrap();
    assert_eq!(vm.eval("face.status").unwrap(), "loaded");
    assert_eq!(vm.eval("JSON.stringify(results)").unwrap(), "[true,true]");
}

#[test]
fn document_resources_load_fonts_even_when_layout_is_disabled() {
    let mut vm = automatic_vm();
    vm.set_layout_policy(moli_page_types::LayoutPolicy::Mock);
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
fn font_discovery_reuses_clean_inputs_and_rechecks_cssom_changes() {
    let mut vm = automatic_vm();
    let (_, wide) = order_font_data();
    vm.eval(&format!(r#"
        globalThis.face=new FontFace('LaterFace','url(data:font/ttf;base64,{wide})');document.fonts.add(face);
        globalThis.result=[];face.loaded.then(()=>result.push('loaded'));
        globalThis.style=document.createElement('style');document.head.append(style);
    "#)).unwrap();
    vm.prime_document_lifecycle_processing_and_record_stylesheet_network_results();
    let passes = vm.layout_pass_observability_for_test().1;
    let discoveries = discovery_count(&mut vm);
    for _ in 0..3 {
        vm.prime_document_lifecycle_processing_and_record_stylesheet_network_results();
    }
    assert_eq!(vm.layout_pass_observability_for_test().1, passes);
    assert_eq!(discovery_count(&mut vm), discoveries);
    assert_eq!(vm.eval("JSON.stringify(result)").unwrap(), "[]");
    vm.eval("style.sheet.insertRule('#probe {font-family:LaterFace !important}',0)")
        .unwrap();
    vm.prime_document_lifecycle_processing_and_record_stylesheet_network_results();
    run_font_tasks(&mut vm);
    assert_eq!(vm.eval("JSON.stringify(result)").unwrap(), r#"["loaded"]"#);
    refresh_layout_for_test(&mut vm);
    assert_eq!(
        vm.eval("String(probe.getBoundingClientRect().width)")
            .unwrap(),
        "160"
    );
}

#[test]
fn child_font_discovery_rechecks_parent_cssom_viewport_changes() {
    let mut vm = automatic_vm();
    vm.set_layout_policy(moli_page_types::LayoutPolicy::Mock);
    let (_, wide) = order_font_data();
    vm.eval(&format!(r#"
        globalThis.parentStyle=document.createElement('style');
        parentStyle.textContent='iframe {{width:200px;height:150px}}';document.head.append(parentStyle);
        globalThis.child=document.getElementById('frame').contentDocument;
        child.head.innerHTML='<style>span {{font-family:serif}} @media (min-width:500px) {{span {{font-family:LateFace}}}}</style>';
        child.body.innerHTML='<span id="text">AAAA</span>';
        globalThis.face=new FontFace('LateFace','url(data:font/ttf;base64,{wide})');
        child.fonts.add(face);
    "#)).unwrap();
    vm.reconcile_document_font_resources();
    run_font_tasks(&mut vm);
    assert_eq!(vm.eval("face.status").unwrap(), "unloaded");
    let before = discovery_count(&mut vm);
    let dom_version = vm._context_host.borrow().dom_host().dom_version();
    let visual_generation = vm._context_host.borrow().visual_state_generation();
    vm.eval("parentStyle.sheet.insertRule('iframe {width:600px}',1)")
        .unwrap();
    assert_eq!(
        vm._context_host.borrow().dom_host().dom_version(),
        dom_version
    );
    assert_eq!(
        vm._context_host.borrow().visual_state_generation(),
        visual_generation
    );
    vm.reconcile_document_font_resources();
    run_font_tasks(&mut vm);
    assert_eq!(vm.eval("face.status").unwrap(), "loaded");
    assert!(discovery_count(&mut vm) > before);
    assert_eq!(vm.layout_pass_observability_for_test().1, 0);
}

#[test]
fn child_font_ready_observes_parent_cssom_in_the_same_script() {
    let mut vm = automatic_vm();
    vm.set_layout_policy(moli_page_types::LayoutPolicy::Mock);
    let (_, wide) = order_font_data();
    vm.eval(&format!(r#"
        globalThis.parentStyle=document.createElement('style');
        parentStyle.textContent='iframe {{width:200px}}';document.head.append(parentStyle);
        globalThis.child=document.getElementById('frame').contentDocument;
        child.head.innerHTML='<style>span {{font-family:serif}} @media (min-width:500px) {{span {{font-family:LateFace}}}}</style>';
        child.body.innerHTML='<span>AAAA</span>';
        globalThis.face=new FontFace('LateFace','url(data:font/ttf;base64,{wide})');child.fonts.add(face);
        globalThis.states=[];
    "#)).unwrap();
    vm.reconcile_document_font_resources();
    run_font_tasks(&mut vm);
    assert_eq!(vm.eval("face.status").unwrap(), "unloaded");
    vm.eval(
        r#"
        parentStyle.sheet.insertRule('iframe {width:600px}',1);
        child.fonts.ready.then(()=>states.push(face.status));
    "#,
    )
    .unwrap();
    run_font_tasks(&mut vm);
    assert_eq!(vm.eval("face.status").unwrap(), "loaded");
    assert_eq!(vm.eval("JSON.stringify(states)").unwrap(), r#"["loaded"]"#);
    assert_eq!(vm.layout_pass_observability_for_test().1, 0);
}

#[test]
fn resources_load_used_faces_without_loading_unused_hidden_empty_or_range_misses() {
    let mut vm = automatic_vm();
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
    refresh_layout_for_test(&mut vm);
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
fn implicit_load_errors_preserve_observed_promises_and_font_set_events() {
    for observe in [false, true] {
        let mut vm = automatic_vm();
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
        vm.reconcile_document_font_resources();
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
fn resources_match_capabilities_before_unicode_ranges() {
    let mut vm = automatic_vm();
    let (_, wide) = order_font_data();
    vm.eval(&format!(r#"
        globalThis.regular=new FontFace('OrderFace','url(data:font/ttf;base64,{wide})',{{weight:'400',unicodeRange:'U+4E00'}});
        globalThis.bold=new FontFace('OrderFace','url(data:font/ttf;base64,{wide})',{{weight:'700',unicodeRange:'U+0041'}});
        document.fonts.add(regular);document.fonts.add(bold);
    "#)).unwrap();
    vm.reconcile_document_font_resources();
    assert_eq!(
        vm.eval("[regular.status,bold.status].join(',')").unwrap(),
        "unloaded,unloaded"
    );
    vm.eval("probe.style.fontWeight='700'").unwrap();
    vm.reconcile_document_font_resources();
    run_font_tasks(&mut vm);
    assert_eq!(
        vm.eval("[regular.status,bold.status].join(',')").unwrap(),
        "unloaded,loaded"
    );
    refresh_layout_for_test(&mut vm);
    assert_eq!(
        vm.eval("String(probe.getBoundingClientRect().width)")
            .unwrap(),
        "160"
    );
}

#[test]
fn font_resource_requests_run_as_window_owned_tasks() {
    let mut vm = automatic_vm();
    let (_, wide) = order_font_data();
    vm.eval(&format!("globalThis.face=new FontFace('OrderFace','url(data:font/ttf;base64,{wide})');document.fonts.add(face)")).unwrap();
    vm.reconcile_document_font_resources();
    assert!(
        vm.has_ready_timeout(),
        "resource processing must schedule a load without another JS read"
    );
    run_font_tasks(&mut vm);
    refresh_layout_for_test(&mut vm);
    assert_eq!(
        vm.eval("[face.status,probe.getBoundingClientRect().width].join(',')")
            .unwrap(),
        "loaded,160"
    );
}

#[test]
fn font_set_ready_observes_resource_requests_before_their_tasks_run() {
    let mut vm = automatic_vm();
    let (_, wide) = order_font_data();
    vm.eval(&format!("globalThis.face=new FontFace('OrderFace','url(data:font/ttf;base64,{wide})');document.fonts.add(face)")).unwrap();
    vm.reconcile_document_font_resources();
    vm.eval("globalThis.ready=false;document.fonts.ready.then(()=>ready=true)")
        .unwrap();
    assert_eq!(vm.eval("String(ready)").unwrap(), "false");
    run_font_tasks(&mut vm);
    refresh_layout_for_test(&mut vm);
    assert_eq!(
        vm.eval("JSON.stringify([ready,face.status,probe.getBoundingClientRect().width])")
            .unwrap(),
        r#"[true,"loaded",160]"#
    );
}

#[test]
fn resources_keep_lower_priority_faces_unloaded_until_they_are_selected() {
    let mut vm = automatic_vm();
    let (narrow, wide) = order_font_data();
    vm.eval(&format!(r#"
        globalThis.pending=new FontFace('OrderFace','url(data:font/ttf;base64,{wide})');
        globalThis.loaded=new FontFace('OrderFace',Uint8Array.from(atob('{narrow}'),c=>c.charCodeAt(0)));
        document.fonts.add(pending);document.fonts.add(loaded);
    "#)).unwrap();
    vm.reconcile_document_font_resources();
    run_font_tasks(&mut vm);
    refresh_layout_for_test(&mut vm);
    assert_eq!(
        vm.eval("[pending.status,probe.getBoundingClientRect().width].join(',')")
            .unwrap(),
        "unloaded,80"
    );
    vm.eval("document.fonts.delete(pending);document.fonts.add(pending)")
        .unwrap();
    vm.reconcile_document_font_resources();
    run_font_tasks(&mut vm);
    refresh_layout_for_test(&mut vm);
    assert_eq!(
        vm.eval("[pending.status,probe.getBoundingClientRect().width].join(',')")
            .unwrap(),
        "loaded,160"
    );
}

#[test]
fn geometry_and_screenshots_do_not_start_or_advance_font_loads() {
    let mut vm = automatic_vm();
    let (_, wide) = order_font_data();
    vm.eval(&format!(r#"
        globalThis.face=new FontFace('OrderFace','url(data:font/ttf;base64,{wide})');document.fonts.add(face);
        globalThis.result=[];face.loaded.then(value=>result.push(value===face));
        Promise.resolve().then(()=>{{void probe.getBoundingClientRect()}});
    "#)).unwrap();
    refresh_layout_for_test(&mut vm);
    publish_layout_for_test(&mut vm);
    assert_eq!(
        vm.eval("JSON.stringify([face.status,result])").unwrap(),
        r#"["unloaded",[]]"#
    );
    assert!(
        !vm.has_ready_timeout(),
        "layout must not enqueue font resource work"
    );
    vm.reconcile_document_font_resources();
    run_font_tasks(&mut vm);
    assert_eq!(
        vm.eval("JSON.stringify([face.status,result])").unwrap(),
        r#"["loaded",[true]]"#
    );
}

#[test]
fn another_document_requests_a_shared_face_in_its_creation_window() {
    let mut vm = automatic_vm();
    let (_, wide) = order_font_data();
    vm.eval(&format!(r#"
        globalThis.face=new FontFace('ChildDemand','url(data:font/ttf;base64,{wide})');
        const child=document.getElementById('frame').contentDocument;
        child.body.innerHTML='<span id="childProbe" style="display:inline-block;font:40px ChildDemand">AAAA</span>';
        child.fonts.add(face);
        globalThis.result=[];face.loaded.then(value=>result.push(value===face));
    "#)).unwrap();
    vm.reconcile_document_font_resources();
    run_font_tasks(&mut vm);
    refresh_layout_for_test(&mut vm);
    assert_eq!(vm.eval("JSON.stringify([face.status,result,document.fonts.has(face),document.getElementById('frame').contentDocument.getElementById('childProbe').getBoundingClientRect().width])").unwrap(), r#"["loaded",[true],false,160]"#);
}

#[tokio::test]
async fn implicit_font_network_load_waits_for_completion_and_font_set_ready() {
    let (url, mut request_rx, release_tx, server) = spawn_gated_font_resource_server().await;
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
        globalThis.disabledReady=false;
        document.fonts.ready.then(()=>disabledReady=true);
    "#)).unwrap();
    vm.prime_document_lifecycle_processing_and_record_stylesheet_network_results();
    run_font_tasks(&mut vm);
    assert_eq!(
        vm.eval("JSON.stringify([face.status,document.fonts.status,disabledReady,result])")
            .unwrap(),
        r#"["unloaded","loaded",true,[]]"#
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_subresource_request_count(),
        0
    );
    assert!(matches!(
        request_rx.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));
    assert_eq!(discovery_count(&mut vm), 0);

    loader.set_optional_resource_fetch_mask(OptionalResourceFetchMask::FONT);
    vm.reconcile_document_font_resources();
    vm.eval(
        r#"
        document.fonts.ready.then(value=>result.push(value===document.fonts));
    "#,
    )
    .unwrap();
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
    refresh_layout_for_test(&mut vm);
    assert_eq!(vm.eval("JSON.stringify([face.status,result,events,document.fonts.status,probe.getBoundingClientRect().width])").unwrap(), r#"["loaded",[true,true],[1,true],"loaded",96]"#);
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_subresource_request_count(),
        0
    );
}

fn discovery_count(vm: &mut ScriptVm) -> u64 {
    vm.with_default_context_scope(|scope, _| {
        Ok(crate::context_bootstrap::font_request_discovery_count(
            scope,
        ))
    })
    .unwrap()
}

#[test]
fn resources_recheck_live_control_values_and_shadow_slot_text_without_layout() {
    let mut vm = automatic_vm();
    vm.set_layout_policy(moli_page_types::LayoutPolicy::Mock);
    let (_, wide) = order_font_data();
    vm.eval(&format!(r#"
        const source='url(data:font/ttf;base64,{wide})';
        globalThis.controlFace=new FontFace('ControlFace',source,{{unicodeRange:'U+0041'}});
        globalThis.shadowFace=new FontFace('ShadowFace',source);
        globalThis.unslottedFace=new FontFace('UnslottedFace',source);
        for(const face of [controlFace,shadowFace,unslottedFace]) document.fonts.add(face);
        globalThis.input=document.createElement('input');input.style.fontFamily='ControlFace';input.value='B';document.body.append(input);
        const host=document.createElement('div');document.body.append(host);
        host.innerHTML='<span slot="used" style="font-family:ShadowFace">AAAA</span><span style="font-family:UnslottedFace">AAAA</span>';
        host.attachShadow({{mode:'open'}}).innerHTML='<slot name="used"></slot>';
    "#)).unwrap();
    vm.reconcile_document_font_resources();
    run_font_tasks(&mut vm);
    assert_eq!(
        vm.eval("[controlFace.status,shadowFace.status,unslottedFace.status].join(',')")
            .unwrap(),
        "unloaded,loaded,unloaded"
    );
    vm.eval("input.value='A'").unwrap();
    vm.reconcile_document_font_resources();
    run_font_tasks(&mut vm);
    assert_eq!(vm.eval("controlFace.status").unwrap(), "loaded");
    assert_eq!(vm.layout_pass_observability_for_test().1, 0);
}

#[test]
fn resources_match_generated_and_transformed_text() {
    let mut vm = automatic_vm();
    vm.set_layout_policy(moli_page_types::LayoutPolicy::Mock);
    let (_, wide) = order_font_data();
    vm.eval(&format!(r#"
        globalThis.face=new FontFace('GeneratedFace','url(data:font/ttf;base64,{wide})',{{unicodeRange:'U+0041'}});document.fonts.add(face);
        const style=document.createElement('style');style.textContent='#probe::before {{content:"a";font-family:GeneratedFace;text-transform:uppercase}}';document.head.append(style);
    "#)).unwrap();
    vm.reconcile_document_font_resources();
    run_font_tasks(&mut vm);
    assert_eq!(vm.eval("face.status").unwrap(), "loaded");
    assert_eq!(vm.layout_pass_observability_for_test().1, 0);
}

#[test]
fn font_requests_follow_text_and_descriptor_changes_without_geometry() {
    let mut vm = automatic_vm();
    vm.set_layout_policy(moli_page_types::LayoutPolicy::Mock);
    let (_, wide) = order_font_data();
    vm.eval(&format!(r#"
        globalThis.face=new FontFace('LaterFace','url(data:font/ttf;base64,{wide})',{{unicodeRange:'U+0042'}});
        document.fonts.add(face);
    "#)).unwrap();
    vm.reconcile_document_font_resources();
    run_font_tasks(&mut vm);
    assert_eq!(vm.eval("face.status").unwrap(), "unloaded");
    vm.eval("face.family='OrderFace'").unwrap();
    vm.reconcile_document_font_resources();
    run_font_tasks(&mut vm);
    assert_eq!(vm.eval("face.status").unwrap(), "unloaded");
    vm.eval("probe.firstChild.data='BBBB'").unwrap();
    vm.reconcile_document_font_resources();
    run_font_tasks(&mut vm);
    assert_eq!(vm.eval("face.status").unwrap(), "loaded");
    assert_eq!(vm.layout_pass_observability_for_test().1, 0);
}

#[test]
fn failed_faces_allow_lower_priority_font_resources_to_load() {
    let mut vm = automatic_vm();
    let (_, wide) = order_font_data();
    vm.eval(&format!(
        r#"
        globalThis.fallback=new FontFace('OrderFace','url(data:font/ttf;base64,{wide})');
        globalThis.missing=new FontFace('OrderFace','local("MoliDefinitelyNotAFont-0001345")');
        document.fonts.add(fallback);document.fonts.add(missing);
    "#
    ))
    .unwrap();
    vm.eval("globalThis.readyStates=[];document.fonts.ready.then(()=>readyStates.push([missing.status,fallback.status]))").unwrap();
    assert_eq!(vm.eval("JSON.stringify(readyStates)").unwrap(), "[]");
    run_font_tasks(&mut vm);
    assert_eq!(
        vm.eval("[missing.status,fallback.status].join(',')")
            .unwrap(),
        "error,loaded"
    );
    assert_eq!(
        vm.eval("JSON.stringify(readyStates)").unwrap(),
        r#"[["error","loaded"]]"#
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, 0);
    refresh_layout_for_test(&mut vm);
    assert_eq!(
        vm.eval("String(probe.getBoundingClientRect().width)")
            .unwrap(),
        "160"
    );
}

#[test]
fn font_set_ready_waits_for_all_synchronous_fallback_failures() {
    let (_, wide) = order_font_data();
    for failures in [2, 3] {
        let mut vm = automatic_vm();
        vm.set_layout_policy(moli_page_types::LayoutPolicy::Mock);
        vm.eval(&format!(r#"
            globalThis.fallback=new FontFace('OrderFace','url(data:font/ttf;base64,{wide})');
            globalThis.missing=Array.from({{length:{failures}}},(_,index)=>
                new FontFace('OrderFace',`local("MoliDefinitelyNotAFont-Ready-${{index}}")`));
            document.fonts.add(fallback);
            for(const face of missing) document.fonts.add(face);
            globalThis.events=[];
            document.fonts.addEventListener('loading',()=>events.push('loading'));
            document.fonts.addEventListener('loadingdone',event=>
                events.push(['done',event.fontfaces.length,event.fontfaces[0]===fallback]));
            document.fonts.addEventListener('loadingerror',event=>
                events.push(['error',event.fontfaces.length,event.fontfaces.every(face=>missing.includes(face))]));
            globalThis.readyStates=[];
            globalThis.ready=document.fonts.ready;
            ready.then(()=>readyStates.push([...missing].reverse().map(face=>face.status).concat(fallback.status)));
        "#)).unwrap();
        assert_eq!(vm.eval("JSON.stringify(readyStates)").unwrap(), "[]");
        run_font_tasks(&mut vm);
        let mut expected = vec!["error"; failures];
        expected.push("loaded");
        assert_eq!(
            vm.eval("JSON.stringify(readyStates)").unwrap(),
            serde_json::to_string(&[expected]).unwrap(),
            "{failures} synchronous failures",
        );
        assert_eq!(
            vm.eval("JSON.stringify(events)").unwrap(),
            format!(r#"["loading",["done",1,true],["error",{failures},true]]"#),
        );
        assert_eq!(vm.eval("ready===document.fonts.ready").unwrap(), "true");
        assert_eq!(vm.layout_pass_observability_for_test().1, 0);
    }
}

#[test]
fn automatic_fonts_retired_creation_window_does_not_block_live_fallback() {
    for retirement in ["before-demand", "after-queue", "live"] {
        let mut vm = automatic_vm();
        vm.set_layout_policy(moli_page_types::LayoutPolicy::Mock);
        let (_, wide) = order_font_data();
        vm.eval(&format!(
            r#"
            globalThis.frame=document.getElementById('frame');
            globalThis.creationWindow=frame.contentWindow;
            const source='url(data:font/ttf;base64,{wide})';
            globalThis.fallback=new FontFace('RetiredFace',source);
            globalThis.retired=new creationWindow.FontFace('RetiredFace',source);
            document.fonts.add(fallback);document.fonts.add(retired);
        "#
        ))
        .unwrap();
        if retirement == "after-queue" {
            vm.eval("probe.style.fontFamily='RetiredFace'").unwrap();
            vm.reconcile_document_font_resources();
            assert!(vm.has_ready_timeout());
        }
        if retirement != "live" {
            vm.eval("frame.remove()").unwrap();
        }
        vm.eval("probe.style.fontFamily='RetiredFace'").unwrap();
        // Advance only the automatic lifecycle. FontFaceSet getters can
        // synchronously start a stranded queued face and mask this failure.
        for _ in 0..3 {
            vm.prime_document_lifecycle_processing_and_record_stylesheet_network_results();
            run_font_tasks(&mut vm);
        }
        assert_eq!(
            vm.eval("[retired.status,fallback.status].join(',')")
                .unwrap(),
            if retirement == "live" {
                "loaded,unloaded"
            } else {
                "error,loaded"
            },
            "{retirement}",
        );
        assert_eq!(vm.layout_pass_observability_for_test().1, 0);
    }
}
