use super::*;

fn queue_native_fragment_effects(
    engine: &mut MoliStyleEngine,
    host: &DomHost,
    effects: &crate::dom::native::DomMutationEffects,
) {
    assert!(effects.did_change());
    let effects = StyleMutationEffect::from_dom_mutation_effects(host, effects);
    assert!(!effects.is_empty());
    engine.invalidate_for_mutations(
        host,
        &effects,
        &crate::protocol_types::EmulatedMediaOverrides::default(),
    );
}

// ParentNode.append converts its arguments through an ordinary fragment and
// commits the staging and insertion separately. Do not drain between them:
// the removed node already has its final parent when deferred work is read.
fn insert_through_consumed_fragment(
    host: &mut DomHost,
    engine: &mut MoliStyleEngine,
    parent: DomHandle,
    child: DomHandle,
    before: Option<DomHandle>,
) {
    let fragment = host.create_document_fragment();
    let staged = host.append_child_effects(fragment, child);
    queue_native_fragment_effects(engine, host, &staged);
    let inserted = host.insert_before_effects(parent, fragment, before);
    assert!(host.node(fragment).unwrap().first_child().is_none());
    assert!(!host.is_shadow_root(fragment));
    assert_eq!(host.node(child).unwrap().parent_node(), Some(parent));
    assert!(
        inserted
            .style()
            .child_list_mutations()
            .iter()
            .any(|effect| { effect.target() == fragment && effect.removed_nodes() == [child] })
    );
    queue_native_fragment_effects(engine, host, &inserted);
}

fn fragment_test_styles(
    document: DomHandle,
    css: &str,
) -> (MoliStyleEngine, url::Url, FullStyleWorldSnapshot) {
    let mut engine = MoliStyleEngine::new();
    let url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(css.into(), url.clone());
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);
    (engine, url, inputs)
}

#[test]
fn consumed_fragment_tail_append_preserves_existing_canonical_styles() {
    let mut host = test_host();
    let document = host.document_handle();
    let html = host.create_element("html");
    let body = host.create_element("body");
    let list = host.create_element("div");
    let unrelated = host.create_element("aside");
    assert!(host.append_child(document, html));
    assert!(host.append_child(html, body));
    assert!(host.append_child(body, list));
    assert!(host.append_child(body, unrelated));
    let old_items: [DomHandle; 2] = std::array::from_fn(|_| {
        let item = host.create_element("div");
        let text = host.create_text_node("ABCD");
        assert!(host.set_attribute(item, "class", "item"));
        assert!(host.append_child(item, text));
        assert!(host.append_child(list, item));
        item
    });
    let (mut engine, url, inputs) = fragment_test_styles(
        document,
        ".item { font: 16px/20px monospace; color: rgb(1, 2, 3); }",
    );
    let retained = [html, body, list, old_items[0], old_items[1], unrelated].map(|handle| {
        let snapshot = computed_style_snapshot_for_test(&engine, &host, &url, handle, &inputs);
        (handle, snapshot.computed_values())
    });

    for _ in 0..2 {
        let item = host.create_element("div");
        let text = host.create_text_node("ABCD");
        assert!(host.set_attribute(item, "class", "item"));
        assert!(host.append_child(item, text));
        insert_through_consumed_fragment(&mut host, &mut engine, list, item, None);
    }
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);
    let resolutions = engine.element_style_resolution_count_for_document_for_test(document);
    for (handle, before) in retained {
        let after = computed_style_snapshot_for_test(&engine, &host, &url, handle, &inputs);
        assert!(
            ServoArc::ptr_eq(&before, &after.computed_values()),
            "tail append through a consumed ordinary fragment must keep existing style {handle:?}"
        );
        let canonical = retained_primary_style_for_test(&engine, &host, handle).unwrap();
        assert!(ServoArc::ptr_eq(&before, &canonical));
    }
    assert_eq!(
        engine.element_style_resolution_count_for_document_for_test(document),
        resolutions,
        "reading unaffected styles must not recascade them"
    );
}

#[test]
fn consumed_fragment_insertion_still_invalidates_structural_selectors() {
    const BASE: &str = "rgb(1, 2, 3)";
    const MATCHED: &str = "rgb(9, 8, 7)";
    // The values are sampled by stable handles, not by changing sibling index.
    // The sibling case inserts before the second item; the other cases append.
    for (selector, before_values, after_values, insert_before_second, changed) in [
        (
            ".item:last-child",
            [BASE, MATCHED],
            [BASE, BASE, MATCHED],
            false,
            1,
        ),
        (
            ".item:nth-last-child(2)",
            [MATCHED, BASE],
            [BASE, MATCHED, BASE],
            false,
            0,
        ),
        (
            "#list:has(> .tail) > .item",
            [BASE, BASE],
            [MATCHED, MATCHED, MATCHED],
            false,
            0,
        ),
        (
            ".tail + .item",
            [BASE, BASE],
            [BASE, MATCHED, BASE],
            true,
            1,
        ),
    ] {
        let mut host = test_host();
        let document = host.document_handle();
        let html = host.create_element("html");
        let list = host.create_element("div");
        assert!(host.set_attribute(list, "id", "list"));
        assert!(host.append_child(document, html));
        assert!(host.append_child(html, list));
        let old_items: [DomHandle; 2] = std::array::from_fn(|_| {
            let item = host.create_element("div");
            assert!(host.set_attribute(item, "class", "item"));
            assert!(host.append_child(list, item));
            item
        });
        let css = format!(
            ".item {{ background-color: {BASE}; }} {selector} {{ background-color: {MATCHED}; }}"
        );
        let (mut engine, url, inputs) = fragment_test_styles(document, &css);
        let read_color = |engine: &MoliStyleEngine, host: &DomHost, handle| {
            engine
                .computed_style_property_value(
                    host,
                    &url,
                    handle,
                    "background-color",
                    None,
                    &inputs,
                    None,
                )
                .unwrap()
        };
        assert_eq!(
            old_items.map(|item| read_color(&engine, &host, item)),
            before_values,
            "initial selector precondition: {selector}"
        );
        let before = retained_primary_style_for_test(&engine, &host, old_items[changed]).unwrap();
        let added = host.create_element("div");
        assert!(host.set_attribute(added, "class", "item tail"));
        insert_through_consumed_fragment(
            &mut host,
            &mut engine,
            list,
            added,
            insert_before_second.then_some(old_items[1]),
        );
        engine.drain_pending_style_invalidations_for_document_for_test(&host, document);
        assert_eq!(
            [old_items[0], old_items[1], added].map(|item| read_color(&engine, &host, item)),
            after_values,
            "the final insertion must preserve real selector invalidation: {selector}"
        );
        let after = retained_primary_style_for_test(&engine, &host, old_items[changed]).unwrap();
        assert!(
            !ServoArc::ptr_eq(&before, &after),
            "an existing element whose computed value changed must be republished: {selector}"
        );
    }
}
