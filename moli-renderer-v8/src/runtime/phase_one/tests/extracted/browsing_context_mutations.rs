use super::*;

#[test]
fn parser_mutation_owner_syncs_inserted_child_browsing_context_without_driver_resync() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
        let mut page_vm = new_phase_one_page_vm_for_test();
        let context_host = page_vm
            .vm()
            .context_host_weak_for_test()
            .upgrade()
            .expect("context host should be alive");
        assert!(
            context_host
                .borrow_mut()
                .take_pending_child_frame_tree_events()
                .is_empty(),
            "test setup should start without pending child frame attachments"
        );
        let (body, iframe) = {
            let body = create_connected_html_body_for_test(&mut page_vm);
            let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
            let iframe = dom_host.create_parser_element_without_attributes(
                "iframe".to_owned(),
                "http://www.w3.org/1999/xhtml".to_owned(),
                None,
            );
            (body, iframe)
        };
        assert!(
            context_host
                .borrow_mut()
                .take_pending_child_frame_tree_events()
                .is_empty(),
            "creating a disconnected parser iframe should not create a child context"
        );

        let custom_element_reaction_roots = {
            apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::AppendChild {
                    parent: body,
                    child: iframe,
                },
                "parser DOM mutation should apply",
            )
        };
        assert!(
            custom_element_reaction_roots.is_empty(),
            "plain iframe insertion should not queue custom element reactions"
        );
        let attachments = context_host
            .borrow_mut()
            .take_pending_child_frame_tree_events();
        assert_eq!(
            attachments.len(),
            1,
            "parser insertion followup should attach the inserted iframe immediately"
        );
        assert!(matches!(
            &attachments[0],
            crate::protocol_types::ChildFrameTreeEventSnapshot::Attached(attachment)
                if attachment.parent_frame_id.is_none()
        ));
    }));
}
#[test]
fn parser_document_fragment_append_child_syncs_child_browsing_context_subtree() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            let body = create_connected_html_body_for_test(&mut page_vm);
            let context_host = page_vm
                .vm()
                .context_host_weak_for_test()
                .upgrade()
                .expect("context host should be alive");
            assert!(
                context_host
                    .borrow_mut()
                    .take_pending_child_frame_tree_events()
                    .is_empty(),
                "test setup should start without pending child frame attachments"
            );
            let (fragment, container, iframe) = {
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                let fragment = dom_host.create_document_fragment();
                let container = dom_host.create_parser_element_without_attributes(
                    "section".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(container, "id", "parser-fragment-iframe-root"));
                let iframe = dom_host.create_parser_element_without_attributes(
                    "iframe".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(iframe, "id", "parser-fragment-iframe"));
                assert!(dom_host.append_child(container, iframe));
                assert!(dom_host.append_child(fragment, container));
                (fragment, container, iframe)
            };
            assert!(
                context_host
                    .borrow_mut()
                    .take_pending_child_frame_tree_events()
                    .is_empty(),
                "creating a disconnected parser fragment iframe subtree should not create a child context"
            );

            let custom_element_reaction_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::AppendChild {
                        parent: body,
                        child: fragment,
                    },
                    "parser fragment iframe appendChild should apply",
                )
            };
            assert!(
                custom_element_reaction_roots.is_empty(),
                "plain iframe fragment append should not queue custom element reactions"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .child_handles(fragment)
                    .count(),
                0,
                "parser DocumentFragment iframe append should hoist and empty the fragment"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .child_handles(body)
                    .collect::<Vec<_>>(),
                vec![container],
                "parser DocumentFragment iframe append should append the hoisted root"
            );
            assert!(
                context_host
                    .borrow()
                    .child_browsing_context_frame_id_by_owner_node_id(iframe)
                    .is_some(),
                "parser DocumentFragment append followup should register iframe subtrees under hoisted roots"
            );
            let attachments = context_host
                .borrow_mut()
                .take_pending_child_frame_tree_events();
            assert_eq!(
                attachments.len(),
                1,
                "parser DocumentFragment append followup should attach the iframe subtree immediately"
            );
            assert!(matches!(
                &attachments[0],
                crate::protocol_types::ChildFrameTreeEventSnapshot::Attached(attachment)
                    if attachment.parent_frame_id.is_none()
            ));
        }));
}
#[test]
fn parser_document_fragment_insert_before_syncs_child_browsing_context_subtree() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            let body = create_connected_html_body_for_test(&mut page_vm);
            let context_host = page_vm
                .vm()
                .context_host_weak_for_test()
                .upgrade()
                .expect("context host should be alive");
            assert!(
                context_host
                    .borrow_mut()
                    .take_pending_child_frame_tree_events()
                    .is_empty(),
                "test setup should start without pending child frame attachments"
            );
            let (fragment, container, iframe, reference) = {
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                let reference = dom_host.create_parser_element_without_attributes(
                    "span".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(reference, "id", "parser-fragment-iframe-ref"));
                assert!(dom_host.append_child(body, reference));

                let fragment = dom_host.create_document_fragment();
                let container = dom_host.create_parser_element_without_attributes(
                    "section".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(container, "id", "parser-fragment-iframe-root"));
                let iframe = dom_host.create_parser_element_without_attributes(
                    "iframe".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(iframe, "id", "parser-fragment-iframe"));
                assert!(dom_host.append_child(container, iframe));
                assert!(dom_host.append_child(fragment, container));
                (fragment, container, iframe, reference)
            };
            assert!(
                context_host
                    .borrow_mut()
                    .take_pending_child_frame_tree_events()
                    .is_empty(),
                "creating a disconnected parser fragment iframe subtree should not create a child context"
            );

            let custom_element_reaction_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::InsertBefore {
                        parent: body,
                        child: fragment,
                        reference_child: Some(reference),
                    },
                    "parser fragment iframe insertBefore should apply",
                )
            };
            assert!(
                custom_element_reaction_roots.is_empty(),
                "plain iframe fragment insertion should not queue custom element reactions"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .child_handles(fragment)
                    .count(),
                0,
                "parser DocumentFragment iframe insertion should hoist and empty the fragment"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .child_handles(body)
                    .collect::<Vec<_>>(),
                vec![container, reference],
                "parser DocumentFragment iframe insertion should place the hoisted root before the reference child"
            );
            assert!(
                context_host
                    .borrow()
                    .child_browsing_context_frame_id_by_owner_node_id(iframe)
                    .is_some(),
                "parser DocumentFragment insertion followup should register iframe subtrees under hoisted roots"
            );
            let attachments = context_host
                .borrow_mut()
                .take_pending_child_frame_tree_events();
            assert_eq!(
                attachments.len(),
                1,
                "parser DocumentFragment insertion followup should attach the iframe subtree immediately"
            );
            assert!(matches!(
                &attachments[0],
                crate::protocol_types::ChildFrameTreeEventSnapshot::Attached(attachment)
                    if attachment.parent_frame_id.is_none()
            ));
        }));
}
#[test]
fn parser_document_fragment_append_child_clears_disconnected_shadow_roots_in_subtree() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            let body = create_connected_html_body_for_test(&mut page_vm);

            let connected = page_vm
                .evaluate_expression(
                    r#"
(() => {
  const host = document.createElement('div');
  host.id = 'parser-fragment-shadow-append-host';
  document.body.appendChild(host);
  const shadow = host.attachShadow({ mode: 'open' });
  const target = document.createElement('span');
  target.id = 'parser-fragment-shadow-append-target';
  target.className = 'target';
  shadow.appendChild(target);
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('.target { color: rgb(0, 128, 0); }');
  shadow.adoptedStyleSheets = [sheet];
  window.parserFragmentShadowAppendHost = host;
  window.parserFragmentShadowAppendTarget = target;
  return getComputedStyle(target).color;
})()
"#,
                )
                .expect("shadow append host setup should evaluate");
            assert_eq!(
                connected.get("value").and_then(serde_json::Value::as_str),
                Some("rgb(0, 128, 0)"),
                "connected shadow tree style should apply before removal"
            );

            let host = page_vm
                .vm()
                .document_runtime
                .get_element_by_id("parser-fragment-shadow-append-host")
                .expect("connected shadow host should exist");

            let disconnected = page_vm
                .evaluate_expression(
                    r#"
(() => {
  window.parserFragmentShadowAppendHost.remove();
  const style = getComputedStyle(window.parserFragmentShadowAppendTarget);
  return JSON.stringify({
    color: style.color,
    length: style.length
  });
})()
"#,
                )
                .expect("shadow append host removal should evaluate");
            assert_eq!(
                disconnected
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                Some(r#"{"color":"","length":0}"#),
                "removed shadow tree style should be unavailable while disconnected"
            );

            let fragment = {
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                let fragment = dom_host.create_document_fragment();
                assert!(dom_host.append_child(fragment, host));
                fragment
            };

            let custom_element_reaction_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::AppendChild {
                        parent: body,
                        child: fragment,
                    },
                    "parser fragment shadow host appendChild should apply",
                )
            };
            assert!(
                custom_element_reaction_roots.is_empty(),
                "plain shadow host fragment append should not queue custom element reactions"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .child_handles(fragment)
                    .count(),
                0,
                "parser DocumentFragment shadow append should hoist and empty the fragment"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .child_handles(body)
                    .collect::<Vec<_>>(),
                vec![host],
                "parser DocumentFragment shadow append should append the hoisted host"
            );

            let reconnected = page_vm
                .evaluate_expression(
                    "getComputedStyle(window.parserFragmentShadowAppendTarget).color",
                )
                .expect("shadow append host reconnected style should evaluate");
            assert_eq!(
                reconnected.get("value").and_then(serde_json::Value::as_str),
                Some("rgb(0, 128, 0)"),
                "parser DocumentFragment append should clear disconnected shadow-root style markers for hoisted subtrees"
            );
        }));
}
#[test]
fn parser_document_fragment_insert_before_clears_disconnected_shadow_roots_in_subtree() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            let body = create_connected_html_body_for_test(&mut page_vm);

            let connected = page_vm
                .evaluate_expression(
                    r#"
(() => {
  const host = document.createElement('div');
  host.id = 'parser-fragment-shadow-host';
  document.body.appendChild(host);
  const shadow = host.attachShadow({ mode: 'open' });
  const target = document.createElement('span');
  target.id = 'parser-fragment-shadow-target';
  target.className = 'target';
  shadow.appendChild(target);
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('.target { color: rgb(0, 128, 0); }');
  shadow.adoptedStyleSheets = [sheet];
  window.parserFragmentShadowHost = host;
  window.parserFragmentShadowTarget = target;
  return getComputedStyle(target).color;
})()
"#,
                )
                .expect("shadow host setup should evaluate");
            assert_eq!(
                connected.get("value").and_then(serde_json::Value::as_str),
                Some("rgb(0, 128, 0)"),
                "connected shadow tree style should apply before removal"
            );

            let host = page_vm
                .vm()
                .document_runtime
                .get_element_by_id("parser-fragment-shadow-host")
                .expect("connected shadow host should exist");

            let disconnected = page_vm
                .evaluate_expression(
                    r#"
(() => {
  window.parserFragmentShadowHost.remove();
  const style = getComputedStyle(window.parserFragmentShadowTarget);
  return JSON.stringify({
    color: style.color,
    length: style.length
  });
})()
"#,
                )
                .expect("shadow host removal should evaluate");
            assert_eq!(
                disconnected
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                Some(r#"{"color":"","length":0}"#),
                "removed shadow tree style should be unavailable while disconnected"
            );

            let (fragment, reference) = {
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                let reference = dom_host.create_parser_element_without_attributes(
                    "span".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(reference, "id", "parser-fragment-shadow-ref"));
                assert!(dom_host.append_child(body, reference));
                let fragment = dom_host.create_document_fragment();
                assert!(dom_host.append_child(fragment, host));
                (fragment, reference)
            };

            let custom_element_reaction_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::InsertBefore {
                        parent: body,
                        child: fragment,
                        reference_child: Some(reference),
                    },
                    "parser fragment shadow host insertBefore should apply",
                )
            };
            assert!(
                custom_element_reaction_roots.is_empty(),
                "plain shadow host fragment insertion should not queue custom element reactions"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .child_handles(fragment)
                    .count(),
                0,
                "parser DocumentFragment shadow insertion should hoist and empty the fragment"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .child_handles(body)
                    .collect::<Vec<_>>(),
                vec![host, reference],
                "parser DocumentFragment shadow insertion should place the hoisted host before the reference child"
            );

            let reconnected = page_vm
                .evaluate_expression("getComputedStyle(window.parserFragmentShadowTarget).color")
                .expect("shadow host reconnected style should evaluate");
            assert_eq!(
                reconnected.get("value").and_then(serde_json::Value::as_str),
                Some("rgb(0, 128, 0)"),
                "parser DocumentFragment insertion should clear disconnected shadow-root style markers for hoisted subtrees"
            );
        }));
}
#[test]
fn parser_mutation_owner_drops_removed_child_browsing_context_without_driver_resync() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
        let mut page_vm = new_phase_one_page_vm_for_test();
        let context_host = page_vm
            .vm()
            .context_host_weak_for_test()
            .upgrade()
            .expect("context host should be alive");

        let (body, iframe) = {
            let body = create_connected_html_body_for_test(&mut page_vm);
            let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
            let iframe = dom_host.create_parser_element_without_attributes(
                "iframe".to_owned(),
                "http://www.w3.org/1999/xhtml".to_owned(),
                None,
            );
            (body, iframe)
        };

        {
            apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::AppendChild {
                    parent: body,
                    child: iframe,
                },
                "parser append mutation should apply",
            );
        }
        let frame_id = context_host
            .borrow()
            .child_browsing_context_frame_id_by_owner_node_id(iframe)
            .expect("parser append followup should register the iframe child context");
        assert!(
            !frame_id.is_empty(),
            "registered child frame id should be observable before removal"
        );

        let had_pending_work = apply_parser_dom_mutation_and_run_post_step_work_for_test(
            &mut page_vm,
            ParserDomMutation::RemoveChild {
                parent: body,
                child: iframe,
            },
            "parser remove mutation should apply",
            "parser removal reactions should dispatch",
        );
        assert!(
            had_pending_work,
            "parser iframe removal should defer removed-subtree lifecycle followups"
        );
        assert_eq!(
            context_host
                .borrow()
                .child_browsing_context_frame_id_by_owner_node_id(iframe),
            None,
            "parser removal followup should drop the iframe child context registry entry"
        );
        let frame_tree_events = context_host
            .borrow_mut()
            .take_pending_child_frame_tree_events();
        assert_eq!(
            frame_tree_events.len(),
            2,
            "an iframe inserted and removed before a protocol drain must preserve both tree events"
        );
        assert!(matches!(
            &frame_tree_events[0],
            crate::protocol_types::ChildFrameTreeEventSnapshot::Attached(attachment)
                if attachment.frame_id == frame_id
        ));
        assert!(matches!(
            &frame_tree_events[1],
            crate::protocol_types::ChildFrameTreeEventSnapshot::Detached(detachment)
                if detachment.frame_id == frame_id
        ));
    }));
}
#[test]
fn parser_reparent_to_disconnected_parent_drops_child_browsing_context() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            let context_host = page_vm
                .vm()
                .context_host_weak_for_test()
                .upgrade()
                .expect("context host should be alive");

            let (body, detached_parent, iframe) = {
                let body = create_connected_html_body_for_test(&mut page_vm);
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                let detached_parent = dom_host.create_parser_element_without_attributes(
                    "div".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                let iframe = dom_host.create_parser_element_without_attributes(
                    "iframe".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                (body, detached_parent, iframe)
            };

            {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::AppendChild {
                        parent: body,
                        child: iframe,
                    },
                    "parser append mutation should apply",
                );
            }
            assert!(
                context_host
                    .borrow()
                    .child_browsing_context_frame_id_by_owner_node_id(iframe)
                    .is_some(),
                "parser append followup should register the iframe child context before reparent"
            );

            let had_pending_work = apply_parser_dom_mutation_and_run_post_step_work_for_test(
                &mut page_vm,
                ParserDomMutation::InsertBefore {
                    parent: detached_parent,
                    child: iframe,
                    reference_child: None,
                },
                "parser reparent mutation should apply",
                "parser reparent lifecycle followups should dispatch",
            );
            assert!(
                had_pending_work,
                "parser reparent to a disconnected parent should defer removed-subtree lifecycle followups"
            );
            assert_eq!(
                context_host
                    .borrow()
                    .child_browsing_context_frame_id_by_owner_node_id(iframe),
                None,
                "parser reparent to a disconnected parent should drop the iframe child context"
            );
        }));
}
