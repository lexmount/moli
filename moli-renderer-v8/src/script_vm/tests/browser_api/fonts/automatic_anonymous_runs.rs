use super::automatic_controls::font_usage_vm;
use super::selection_order::order_font_data;
use super::*;

fn assert_anonymous_space_demand(display: &str, whitespace: &str, setup: &str, expected: &str) {
    let mut vm = font_usage_vm();
    let (_, wide) = order_font_data();
    vm.eval(&format!(r#"
        globalThis.face = new FontFace('AnonymousFace', 'url(data:font/ttf;base64,{wide})', {{unicodeRange:'U+0020'}});
        document.fonts.add(face);
        globalThis.events = [];
        for (const type of ['loading', 'loadingdone', 'loadingerror']) {{
            document.fonts.addEventListener(type, () => events.push(type));
        }}
        const container = document.createElement('div');
        container.style.cssText = 'display:{display};white-space:{whitespace};font-family:AnonymousFace';
        document.body.replaceChildren(container);
        {setup}
    "#)).unwrap();
    vm.prime_document_lifecycle_processing_and_record_stylesheet_network_results();
    run_font_tasks(&mut vm);
    assert_eq!(
        vm.eval("face.status").unwrap(),
        expected,
        "{display}/{whitespace}/{setup}"
    );
    vm.eval("document.fonts.ready.then(() => globalThis.ready = true)")
        .unwrap();
    run_font_tasks(&mut vm);
    assert_eq!(vm.eval("globalThis.ready === true").unwrap(), "true");
    assert_eq!(vm.eval("face.status").unwrap(), expected);
    if expected == "unloaded" {
        assert_eq!(vm.eval("JSON.stringify(events)").unwrap(), "[]");
    }
    assert_eq!(vm.layout_pass_observability_for_test().1, 0);
}

#[test]
fn automatic_fonts_drop_only_whitespace_anonymous_flex_and_grid_runs() {
    for display in ["flex", "inline-flex", "grid", "inline-grid"] {
        for whitespace in ["pre", "break-spaces"] {
            for (setup, expected) in [
                ("container.append('  \\t\\n\\r\\f');", "unloaded"),
                (
                    "container.append(' ', document.createComment('boundary'), ' ');",
                    "unloaded",
                ),
                (
                    "container.innerHTML = ' <span style=\"display:none\">A</span> ';",
                    "unloaded",
                ),
                (
                    "container.innerHTML = '<span style=\"display:contents\"> <i style=\"display:contents\"> </i></span>';",
                    "unloaded",
                ),
                ("container.innerHTML = ' <span></span> ';", "unloaded"),
                ("container.innerHTML = ' <span>A</span> ';", "unloaded"),
                (
                    "container.innerHTML = ' <span style=\"position:absolute\">A</span> ';",
                    "unloaded",
                ),
                (
                    "container.innerHTML = ' <span style=\"display:flex\"> </span> ';",
                    "unloaded",
                ),
                (
                    "container.innerHTML = ' <span style=\"display:flex\"> A </span> ';",
                    "loaded",
                ),
                (
                    "container.innerHTML = '<span style=\"display:contents;position:absolute\"> </span>';",
                    "unloaded",
                ),
                (
                    "container.innerHTML = '<span style=\"display:contents;float:left\"> </span>';",
                    "unloaded",
                ),
                (
                    "container.innerHTML = ' <span style=\"display:contents;position:absolute;font-family:serif\">A</span>';",
                    "loaded",
                ),
                ("container.append(' A ');", "loaded"),
                ("container.append(' ', 'A');", "loaded"),
                ("container.append('A', ' ');", "loaded"),
                (
                    "container.innerHTML = ' <span style=\"display:contents;font-family:serif\">A</span>';",
                    "loaded",
                ),
                (
                    "container.innerHTML = '<span style=\"display:contents;font-family:serif\">A</span> ';",
                    "loaded",
                ),
                ("container.innerHTML = '<span> </span>';", "loaded"),
                (
                    "container.innerHTML = '<span style=\"display:contents\"> A </span>';",
                    "loaded",
                ),
                ("container.append(' ', '\\u00a0');", "loaded"),
                (
                    "container.innerHTML = '<input value=\" \" style=\"display:flex;white-space:pre\">'; container.firstChild.style.fontFamily='AnonymousFace';",
                    "loaded",
                ),
            ] {
                assert_anonymous_space_demand(display, whitespace, setup, expected);
            }
        }
    }
}

#[test]
fn automatic_fonts_normalize_each_retained_anonymous_item_independently() {
    for display in ["flex", "grid"] {
        for (setup, expected) in [
            ("container.innerHTML = ' <span></span> A';", "unloaded"),
            ("container.innerHTML = 'A <span></span> ';", "unloaded"),
            ("container.append(' A ');", "unloaded"),
            ("container.append('A', ' ', 'B');", "loaded"),
            (
                "container.innerHTML = 'A<span style=\"display:contents\"> </span>B';",
                "loaded",
            ),
        ] {
            assert_anonymous_space_demand(display, "normal", setup, expected);
        }
    }
}

#[test]
fn automatic_fonts_generated_anonymous_runs_follow_box_participation() {
    for display in ["flex", "grid"] {
        for whitespace in ["pre", "break-spaces"] {
            for pseudo in ["before", "after"] {
                for (pseudo_display, content, text, expected) in [
                    ("contents", "' '", "", "unloaded"),
                    ("contents", "'A'", " ", "loaded"),
                    ("contents", "' '", "A", "loaded"),
                    ("inline", "' '", "", "loaded"),
                    ("flex", "' '", "", "unloaded"),
                    ("grid", "' '", "", "unloaded"),
                    ("contents", "' ' counter(probe)", "", "loaded"),
                ] {
                    let css = serde_json::to_string(&format!(
                        "#generated::{pseudo} {{content:{content};display:{pseudo_display}}}"
                    ))
                    .unwrap();
                    let text = serde_json::to_string(text).unwrap();
                    assert_anonymous_space_demand(
                        display,
                        whitespace,
                        &format!(
                            "container.id='generated'; const style=document.createElement('style'); style.textContent={css}; document.head.append(style); container.append({text});"
                        ),
                        expected,
                    );
                }
            }
        }
    }
}

#[test]
fn automatic_fonts_keep_preserved_spaces_in_flow_and_contents_containers() {
    for display in ["block", "inline", "contents"] {
        for whitespace in ["pre", "break-spaces"] {
            assert_anonymous_space_demand(display, whitespace, "container.append('  ');", "loaded");
        }
    }
}

#[test]
fn font_usage_matches_retained_anonymous_item_text() {
    use std::collections::{BTreeSet, HashSet};

    fn collect_text(node: &moli_layout::NormalizedBoxNode, characters: &mut BTreeSet<char>) {
        if let Some(text) = &node.text {
            characters.extend(text.chars());
        }
        for child in &node.children {
            collect_text(child, characters);
        }
    }

    for display in ["flex", "grid", "block", "contents"] {
        for (markup, item_text, flow_text) in [
            ("  ", "", " "),
            (" <span></span> ", "", " "),
            (" <span>A</span> ", "A", " A"),
            (" <span style='display:contents'>A</span> ", " A", " A"),
            ("<span style='display:contents'> </span>", "", " "),
            (
                "<span style='display:contents;position:absolute'> </span>",
                "",
                " ",
            ),
            (
                "<span style='display:contents;float:left'> </span>",
                "",
                " ",
            ),
            ("<span> </span>", " ", " "),
            (" A ", " A", " A"),
            (" \u{00a0} ", " \u{00a0}", " \u{00a0}"),
        ] {
            let mut vm = font_usage_vm();
            let markup = serde_json::to_string(markup).unwrap();
            vm.eval(&format!(r#"
                document.body.innerHTML = '<div id="container"></div>';
                container.style.cssText = 'display:{display};white-space:pre;font-family:ContractFace';
                container.innerHTML = {markup};
            "#)).unwrap();
            let host = vm._context_host.borrow();
            let document = host.document_handle();
            let root = host
                .dom_host()
                .dom()
                .document_element_handle_for_document(document)
                .unwrap();
            let families = HashSet::from(["contractface".to_owned()]);
            let discovered =
                crate::font_loading::usage::collect_font_uses(&host, document, &families)
                    .into_values()
                    .flatten()
                    .collect::<BTreeSet<_>>();
            assert_eq!(vm.layout_pass_observability_for_test().1, 0);
            let tree =
                crate::layout_renderer::build_normalized_native_box_tree_for_test(&host, root)
                    .unwrap();
            let mut retained = BTreeSet::new();
            collect_text(&tree.root, &mut retained);
            let expected = if matches!(display, "flex" | "grid") {
                item_text
            } else {
                flow_text
            };
            let expected = expected.chars().collect::<BTreeSet<_>>();
            assert_eq!(retained, expected, "box text: {display}/{markup}");
            assert_eq!(discovered, expected, "discovery: {display}/{markup}");
        }
    }
}

#[tokio::test]
async fn automatic_fonts_discarded_anonymous_spaces_do_not_request_or_delay_ready() {
    use crate::protocol_types::OptionalResourceFetchMask;

    for display in ["flex", "grid"] {
        for whitespace in ["pre", "break-spaces"] {
            for contents in [false, true] {
                let (url, mut request_rx, release_tx, server) =
                    spawn_gated_font_resource_server().await;
                let loader =
                    ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
                loader.set_optional_resource_fetch_mask(OptionalResourceFetchMask::FONT);
                let (mut vm, mut completions) =
                    new_parsed_test_vm_with_loader_and_resource_completion_queue(
                        &url.replace("/print-only.woff2", "/page"),
                        "<!doctype html><div id=target></div>",
                        &loader,
                    );
                vm.set_layout_policy(moli_page_types::LayoutPolicy::Mock);
                vm.eval(&format!(r#"
                    globalThis.face = new FontFace('SpaceFace', 'url("{url}")', {{unicodeRange:'U+0020'}});
                    document.fonts.add(face);
                    target.style.cssText = 'display:{display};white-space:{whitespace};font-family:SpaceFace';
                    globalThis.text = document.createElement('span');
                    text.style.display = 'contents';
                    if ({contents}) {{ target.append(text); }} else {{ globalThis.text = target; }}
                    text.append(' ', document.createComment('not an item'), ' ');
                    globalThis.events = [];
                    for (const type of ['loading', 'loadingdone', 'loadingerror']) {{
                        document.fonts.addEventListener(type, () => events.push(type));
                    }}
                "#)).unwrap();
                vm.prime_document_lifecycle_processing_and_record_stylesheet_network_results();
                run_font_tasks(&mut vm);
                assert_eq!(
                    vm.eval("face.status").unwrap(),
                    "unloaded",
                    "{display}/{whitespace}/{contents}"
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
                vm.eval("document.fonts.ready.then(() => globalThis.ready = true)")
                    .unwrap();
                run_font_tasks(&mut vm);
                assert_eq!(
                    vm.eval("JSON.stringify([ready,face.status,events])")
                        .unwrap(),
                    r#"[true,"unloaded",[]]"#
                );
                assert_eq!(vm.layout_pass_observability_for_test().1, 0);

                // Adding non-whitespace retains the whole anonymous item,
                // including its spaces, and must start the previously unused face.
                vm.eval("text.append('A')").unwrap();
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
                assert_eq!(
                    vm.eval("JSON.stringify(events)").unwrap(),
                    r#"["loading","loadingdone"]"#
                );
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
}
