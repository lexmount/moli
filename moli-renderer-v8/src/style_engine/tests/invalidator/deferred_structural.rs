use super::*;
use crate::{dom::native::DomMutationEffects, protocol_types::EmulatedMediaOverrides};

const BASE_COLOR: &str = "rgb(1, 2, 3)";
const HAS_CHILD_COLOR: &str = "rgb(4, 5, 6)";
const EMPTY_BACKGROUND: &str = "rgb(7, 8, 9)";
const TRANSPARENT: &str = "rgba(0, 0, 0, 0)";

struct Fixture {
    host: DomHost,
    engine: MoliStyleEngine,
    url: url::Url,
    inputs: FullStyleWorldSnapshot,
}

impl Fixture {
    fn new(css: &str) -> Self {
        let host = test_host();
        let mut engine = MoliStyleEngine::new();
        let url = url::Url::parse("https://example.test/").unwrap();
        let source = StyloStylesheetSource::new(css.into(), url.clone());
        engine
            .set_document_adopted_style_sheet_sources(host.document_handle(), vec![source.clone()]);
        let mut inputs = FullStyleWorldSnapshot::default();
        inputs.document_stylesheet_sources.push(source);
        Self {
            host,
            engine,
            url,
            inputs,
        }
    }

    fn computed(&mut self, element: DomHandle, property: &str) -> String {
        self.engine
            .computed_style_property_value(
                &self.host,
                &self.url,
                element,
                property,
                None,
                &self.inputs,
                None,
            )
            .expect("element has computed style")
    }

    fn append(&mut self, parent: DomHandle, child: DomHandle) {
        let effects = self.host.append_child_effects(parent, child);
        self.invalidate(&effects);
    }

    fn remove(&mut self, parent: DomHandle, child: DomHandle) {
        let effects = self.host.remove_child_effects(parent, child);
        self.invalidate(&effects);
    }

    fn invalidate(&mut self, effects: &DomMutationEffects) {
        assert!(effects.did_change());
        let style_effects = StyleMutationEffect::from_dom_mutation_effects(effects);
        self.engine.invalidate_for_mutations(
            &self.host,
            &style_effects,
            &EmulatedMediaOverrides::default(),
        );
    }

    fn drain(&self) {
        let document = self.host.document_handle();
        assert_eq!(
            self.engine
                .pending_style_invalidation_work_item_count_for_document_for_test(document),
            1
        );
        self.engine
            .drain_pending_style_invalidations_for_document_for_test(&self.host, document);
        assert_eq!(
            self.engine
                .pending_style_invalidation_work_item_count_for_document_for_test(document),
            0
        );
    }
}

#[test]
fn reconnect_before_drain_updates_has_and_empty_in_both_parents() {
    let mut fixture = Fixture::new(
        "section { color: rgb(1, 2, 3); }
         section:has(> span) { color: rgb(4, 5, 6); }
         section:empty { background-color: rgb(7, 8, 9); }",
    );
    let document = fixture.host.document_handle();
    let main = fixture.host.create_element("main");
    let original = fixture.host.create_element("section");
    let intermediate = fixture.host.create_element("section");
    let destination = fixture.host.create_element("section");
    let moved = fixture.host.create_element("span");
    assert!(fixture.host.append_child(document, main));
    for parent in [original, intermediate, destination] {
        assert!(fixture.host.append_child(main, parent));
    }
    assert!(fixture.host.append_child(original, moved));
    for (element, color, background) in [
        (original, HAS_CHILD_COLOR, TRANSPARENT),
        (intermediate, BASE_COLOR, EMPTY_BACKGROUND),
        (destination, BASE_COLOR, EMPTY_BACKGROUND),
    ] {
        assert_eq!(fixture.computed(element, "color"), color);
        assert_eq!(fixture.computed(element, "background-color"), background);
    }

    fixture.remove(original, moved);
    fixture.append(intermediate, moved);
    fixture.remove(intermediate, moved);
    fixture.append(destination, moved);
    assert_eq!(
        fixture
            .engine
            .pending_structural_style_mutation_effect_count_for_document_for_test(document),
        8
    );
    fixture.drain();

    for (element, color, background) in [
        (original, BASE_COLOR, EMPTY_BACKGROUND),
        (intermediate, BASE_COLOR, EMPTY_BACKGROUND),
        (destination, HAS_CHILD_COLOR, TRANSPARENT),
    ] {
        assert_eq!(fixture.computed(element, "color"), color);
        assert_eq!(fixture.computed(element, "background-color"), background);
    }
}

#[test]
fn refilling_consumed_fragment_before_drain_updates_both_live_scopes() {
    let mut fixture = Fixture::new(
        "section { color: rgb(1, 2, 3); }
         section:has(> span) { color: rgb(4, 5, 6); }
         section:empty { background-color: rgb(7, 8, 9); }",
    );
    let document = fixture.host.document_handle();
    let main = fixture.host.create_element("main");
    let source = fixture.host.create_element("section");
    let destination = fixture.host.create_element("section");
    let refill = fixture.host.create_element("span");
    let inserted = fixture.host.create_element("span");
    let fragment = fixture.host.create_document_fragment();
    assert!(fixture.host.append_child(document, main));
    assert!(fixture.host.append_child(main, source));
    assert!(fixture.host.append_child(main, destination));
    assert!(fixture.host.append_child(source, refill));
    assert_eq!(fixture.computed(source, "color"), HAS_CHILD_COLOR);
    assert_eq!(fixture.computed(source, "background-color"), TRANSPARENT);
    assert_eq!(fixture.computed(destination, "color"), BASE_COLOR);
    assert_eq!(
        fixture.computed(destination, "background-color"),
        EMPTY_BACKGROUND
    );

    fixture.append(fragment, inserted);
    fixture.append(destination, fragment);
    assert_eq!(fixture.host.child_handles(fragment).count(), 0);
    fixture.append(fragment, refill);
    assert_eq!(
        fixture.host.child_handles(fragment).collect::<Vec<_>>(),
        [refill]
    );
    fixture.drain();

    assert_eq!(fixture.computed(source, "color"), BASE_COLOR);
    assert_eq!(
        fixture.computed(source, "background-color"),
        EMPTY_BACKGROUND
    );
    assert_eq!(fixture.computed(destination, "color"), HAS_CHILD_COLOR);
    assert_eq!(
        fixture.computed(destination, "background-color"),
        TRANSPARENT
    );
}

#[test]
fn batched_fragment_splices_refresh_native_relative_selectors() {
    for insertion_count in [255, 300] {
        let mut fixture = Fixture::new(
            "main { color: rgb(1, 2, 3); }
             main:has(span) .subject { color: rgb(4, 5, 6); }",
        );
        let document = fixture.host.document_handle();
        let main = fixture.host.create_element("main");
        let container = fixture.host.create_element("div");
        let subject = fixture.host.create_element("div");
        assert!(fixture.host.set_attribute(subject, "class", "subject"));
        assert!(fixture.host.append_child(document, main));
        assert!(fixture.host.append_child(main, container));
        assert!(fixture.host.append_child(main, subject));
        assert_eq!(fixture.computed(subject, "color"), BASE_COLOR);

        for _ in 0..insertion_count {
            let fragment = fixture.host.create_document_fragment();
            let span = fixture.host.create_element("span");
            fixture.append(fragment, span);
            fixture.append(container, fragment);
        }
        // Each pair records two root effects and three splices. After
        // pruning there is one combined connection effect and one splice
        // per insertion into the live container.
        assert_eq!(
            fixture
                .engine
                .pending_structural_style_mutation_effect_count_for_document_for_test(document),
            insertion_count * 5
        );
        fixture.drain();
        assert_eq!(fixture.computed(subject, "color"), HAS_CHILD_COLOR);
    }
}

#[test]
fn consecutive_insertions_and_attribute_changes_share_one_pending_invalidation() {
    let mut fixture = Fixture::new(
        ".anchor, #target { color: rgb(1, 2, 3); }
         .anchor:has(+ .marker), #target[data-active] { color: rgb(4, 5, 6); }",
    );
    let document = fixture.host.document_handle();
    let main = fixture.host.create_element("main");
    let parent = fixture.host.create_element("section");
    let anchor = fixture.host.create_element("div");
    let marker = fixture.host.create_element("div");
    let target = fixture.host.create_element("div");
    assert!(fixture.host.set_attribute(anchor, "class", "anchor"));
    assert!(fixture.host.set_attribute(marker, "class", "marker"));
    assert!(fixture.host.set_attribute(target, "id", "target"));
    assert!(fixture.host.append_child(document, main));
    assert!(fixture.host.append_child(main, parent));
    assert!(fixture.host.append_child(main, target));
    for child in [anchor, marker] {
        assert!(fixture.host.append_child(parent, child));
    }
    assert_eq!(fixture.computed(anchor, "color"), HAS_CHILD_COLOR);
    assert_eq!(fixture.computed(target, "color"), BASE_COLOR);

    let first = fixture.host.create_element("div");
    let second = fixture.host.create_element("div");
    let insertion = fixture
        .host
        .insert_before_effects(parent, first, Some(marker));
    fixture.invalidate(&insertion);
    let attribute = fixture
        .host
        .set_attribute_effects(target, "data-active", "");
    fixture.invalidate(&attribute);
    let insertion = fixture
        .host
        .insert_before_effects(parent, second, Some(marker));
    fixture.invalidate(&insertion);
    fixture.drain();

    assert_eq!(fixture.computed(anchor, "color"), BASE_COLOR);
    assert_eq!(fixture.computed(target, "color"), HAS_CHILD_COLOR);
}
