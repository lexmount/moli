use super::automatic_controls::font_usage_vm;
use super::selection_order::order_font_data;
use super::*;

fn assert_participating_text(markup: &str, range: &str, expected: &str) {
    let (mut vm, mut completions) = font_usage_vm();
    let (_, wide) = order_font_data();
    let markup = serde_json::to_string(markup).unwrap();
    vm.eval(&format!(r#"
        globalThis.face=new FontFace('ParticipationFace','url(data:font/ttf;base64,{wide})',{{unicodeRange:'{range}'}});
        document.fonts.add(face);
        const style=document.createElement('style');
        style.textContent='input,textarea,select,button {{font-family:ParticipationFace}} .generated::before,.generated::after {{content:"A"}}';
        document.head.append(style);
        const container=document.createElement('div');container.style.fontFamily='ParticipationFace';
        container.innerHTML={markup};document.body.append(container);
    "#)).unwrap();
    vm.prime_document_lifecycle_processing_and_record_stylesheet_network_results();
    complete_local_font_resources(&mut vm, &mut completions);
    assert_eq!(vm.eval("face.status").unwrap(), expected, "{markup}");
    vm.eval("document.fonts.ready.then(()=>globalThis.ready=true)")
        .unwrap();
    complete_local_font_resources(&mut vm, &mut completions);
    assert_eq!(vm.eval("globalThis.ready===true").unwrap(), "true");
    assert_eq!(vm.layout_pass_observability_for_test().1, 0);
}

#[test]
fn automatic_fonts_content_visibility_respects_property_applicability() {
    for (display, hidden) in [
        ("inline", "loaded"),
        ("contents", "loaded"),
        ("block", "unloaded"),
        ("inline-block", "unloaded"),
        ("flex", "unloaded"),
        ("grid", "unloaded"),
        ("table", "loaded"),
        ("table-cell", "unloaded"),
    ] {
        for (visibility, expected) in [("hidden", hidden), ("visible", "loaded")] {
            assert_participating_text(
                &format!("<div style='display:{display};content-visibility:{visibility}'>A</div>"),
                "U+0041",
                expected,
            );
        }
    }
}

#[test]
fn automatic_fonts_skip_leaf_fallback_and_pseudo_text_but_keep_control_labels() {
    for (markup, expected) in [
        ("<canvas>A</canvas>", "unloaded"),
        ("<canvas class='generated'></canvas>", "unloaded"),
        (
            "<canvas class='generated' style='display:list-item'>A</canvas>",
            "unloaded",
        ),
        ("<canvas style='display:contents'>A</canvas>", "unloaded"),
        ("<canvas><input value='A'></canvas>", "unloaded"),
        ("<progress class='generated'>A</progress>", "unloaded"),
        ("<meter class='generated'>A</meter>", "unloaded"),
        ("<audio class='generated'>A</audio>", "unloaded"),
        ("<video class='generated'>A</video>", "unloaded"),
        ("<iframe class='generated'>A</iframe>", "unloaded"),
        ("<div style='display:table-column'>A</div>", "unloaded"),
        (
            "<div class='generated' style='display:table-column-group'><div>A</div></div>",
            "unloaded",
        ),
        ("<object>A</object>", "loaded"),
        ("<object style='display:contents'>A</object>", "unloaded"),
        ("<button>A</button>", "loaded"),
        ("<button style='display:contents'>A</button>", "loaded"),
        ("<span style='display:contents'>A</span>", "loaded"),
        ("<input value='A'>", "loaded"),
        ("<textarea>A</textarea>", "loaded"),
        ("<select><option label='A'></option></select>", "loaded"),
    ] {
        assert_participating_text(markup, "U+0041", expected);
    }
}

#[test]
fn automatic_fonts_hidden_boxes_preserve_outer_text_boundaries() {
    for (markup, expected) in [
        (
            "<span style='text-transform:capitalize'>a<div style='content-visibility:hidden'>x</div>b</span>",
            "loaded",
        ),
        (
            "<span style='text-transform:capitalize'>a<div style='display:none'>x</div>b</span>",
            "unloaded",
        ),
        (
            "<span style='text-transform:capitalize'>a<span style='display:inline-block;content-visibility:hidden'>x</span>b</span>",
            "unloaded",
        ),
    ] {
        assert_participating_text(markup, "U+0042", expected);
    }
}

#[tokio::test]
async fn automatic_fonts_column_pseudos_do_not_request_discarded_generated_text() {
    use crate::protocol_types::OptionalResourceFetchMask;

    for pseudo in ["before", "after"] {
        for display in ["table-column", "table-column-group"] {
            let (url, mut request_rx, release_tx, server) =
                spawn_gated_font_resource_server().await;
            let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
            loader.set_optional_resource_fetch_mask(OptionalResourceFetchMask::FONT);
            let (mut vm, mut completions) =
                new_parsed_test_vm_with_loader_and_resource_completion_queue(
                    &url.replace("/print-only.woff2", "/page"),
                    "<!doctype html><div id=target></div>",
                    &loader,
                );
            vm.set_layout_policy(moli_page_types::LayoutPolicy::Mock);
            vm.eval(&format!(r#"
                globalThis.face=new FontFace('OnlyPseudo','url("{url}")',{{unicodeRange:'U+0041'}});
                document.fonts.add(face);
                globalThis.style=document.createElement('style');
                style.textContent='#target::{pseudo} {{content:"A";display:{display};font-family:OnlyPseudo}}';
                document.head.append(style);
                globalThis.events=[];
                document.fonts.addEventListener('loading',()=>events.push('loading'));
            "#)).unwrap();
            vm.prime_document_lifecycle_processing_and_record_stylesheet_network_results();
            run_font_tasks(&mut vm);
            assert_eq!(
                vm.eval("face.status").unwrap(),
                "unloaded",
                "{pseudo}/{display}"
            );
            assert_eq!(
                vm._context_host
                    .borrow()
                    .pending_subresource_request_count(),
                0,
            );
            assert!(matches!(
                request_rx.try_recv(),
                Err(tokio::sync::oneshot::error::TryRecvError::Empty)
            ));
            vm.eval("document.fonts.ready.then(()=>globalThis.ready=true)")
                .unwrap();
            run_font_tasks(&mut vm);
            assert_eq!(
                vm.eval("JSON.stringify([ready,face.status,events])")
                    .unwrap(),
                r#"[true,"unloaded",[]]"#
            );
            assert_eq!(vm.layout_pass_observability_for_test().1, 0);

            // The same font and server must become active when the generated
            // text is retained, without a FontFace.load() or FontFaceSet getter.
            vm.eval("style.sheet.cssRules[0].style.display='inline'")
                .unwrap();
            vm.prime_document_lifecycle_processing_and_record_stylesheet_network_results();
            run_font_tasks(&mut vm);
            assert_eq!(vm.eval("face.status").unwrap(), "loading");
            let request = tokio::time::timeout(Duration::from_secs(2), request_rx)
                .await
                .unwrap()
                .unwrap();
            assert!(request.contains("GET /print-only.woff2 "), "{request}");
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
            assert_eq!(vm.eval("face.status").unwrap(), "loaded");
            assert_eq!(vm.eval("JSON.stringify(events)").unwrap(), r#"["loading"]"#);
            assert_eq!(
                vm._context_host
                    .borrow()
                    .pending_subresource_request_count(),
                0
            );
            assert_eq!(vm.layout_pass_observability_for_test().1, 0);
        }
    }
}

#[test]
fn automatic_fonts_column_pseudos_preserve_outer_text_boundaries() {
    for pseudo in ["before", "after"] {
        for display in ["table-column", "table-column-group"] {
            assert_participating_text(
                &format!(
                    "<style>.boundary::{pseudo} {{content:'x';display:{display}}}</style><span style='text-transform:capitalize'>a<span class='boundary'></span>b</span>"
                ),
                "U+0042",
                "loaded",
            );
        }
    }
}

#[test]
fn font_usage_matches_retained_box_text_for_child_box_policy() {
    use std::collections::{BTreeSet, HashSet};

    fn collect_text(node: &moli_layout::NormalizedBoxNode, characters: &mut BTreeSet<char>) {
        if let Some(text) = &node.text {
            characters.extend(text.chars());
        }
        for child in &node.children {
            collect_text(child, characters);
        }
    }

    for (tag, display, pseudo_display, expected) in [
        ("div", "block", "inline", "ABC"),
        ("div", "inline", "inline", "ABC"),
        ("div", "inline", "block", "ABC"),
        ("div", "contents", "inline", "ABC"),
        ("div", "none", "inline", ""),
        ("div", "table-column", "inline", ""),
        ("div", "table-column-group", "inline", ""),
        ("div", "block", "table-column", "A"),
        ("div", "block", "table-column-group", "A"),
        ("div", "contents", "table-column", "A"),
        ("canvas", "block", "inline", ""),
        ("canvas", "contents", "inline", ""),
        ("object", "block", "inline", "ABC"),
        ("object", "contents", "inline", ""),
        ("br", "block", "inline", ""),
        ("br", "contents", "inline", ""),
    ] {
        let (mut vm, _completions) = font_usage_vm();
        vm.eval(&format!(
            r#"
            const target=document.createElement('{tag}');
            target.id='target';target.append(document.createTextNode('A'));
            document.body.replaceChildren(target);
            const style=document.createElement('style');
            style.textContent=`
                #target {{display:{display};font-family:ContractFace}}
                #target::before {{content:"B";display:{pseudo_display}}}
                #target::after {{content:"C";display:{pseudo_display}}}
            `;
            document.head.replaceChildren(style);
        "#
        ))
        .unwrap();
        let host = vm._context_host.borrow();
        let document = host.document_handle();
        let root = host
            .dom_host()
            .dom()
            .document_element_handle_for_document(document)
            .unwrap();
        let families = HashSet::from(["contractface".to_owned()]);
        let discovered = crate::font_loading::usage::collect_font_uses(&host, document, &families)
            .into_values()
            .flatten()
            .collect::<BTreeSet<_>>();
        assert_eq!(vm.layout_pass_observability_for_test().1, 0);

        // Construct boxes only in the test oracle. Both paths must match the
        // independent expected characters, even when they share a policy.
        let tree =
            crate::layout_renderer::build_normalized_native_box_tree_for_test(&host, root).unwrap();
        let mut retained = BTreeSet::new();
        collect_text(&tree.root, &mut retained);
        let expected = expected.chars().collect::<BTreeSet<_>>();
        assert_eq!(
            discovered, expected,
            "discovery: {tag}/{display}/{pseudo_display}"
        );
        assert_eq!(
            retained, expected,
            "box construction: {tag}/{display}/{pseudo_display}"
        );
    }
}
