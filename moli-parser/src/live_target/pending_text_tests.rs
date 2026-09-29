use super::*;
use moli_dom::native::{DomMutationRecordKind, DomStringValue};

fn pump(stream: &crate::DocumentStream, host: &mut DomHost, input: &str) -> DomMutationEffects {
    let mut effects = DomMutationEffects::default();
    let mut consumer = TestMutationEffectCollector {
        host,
        effects: &mut effects,
        panic_on_mutation: false,
        finish_texts: None,
    };
    let outcome = stream
        .pump_parser_step_with_runtime_dom_consumer_without_element_creation(input, &mut consumer);
    assert!(matches!(
        outcome.result,
        crate::ParserPumpStep::InputDrained
    ));
    effects
}

fn old_texts(effects: &DomMutationEffects) -> Vec<DomStringValue> {
    effects
        .observer_records()
        .records()
        .iter()
        .filter_map(|record| match record.kind() {
            DomMutationRecordKind::CharacterData { old_value } => old_value.clone(),
            _ => None,
        })
        .collect()
}

fn assert_old_texts(effects: &DomMutationEffects, expected: &[&str]) {
    let actual = old_texts(effects);
    assert_eq!(actual.len(), expected.len(), "character-data record count");
    assert_eq!(
        actual,
        expected
            .iter()
            .map(|text| DomStringValue::from(*text))
            .collect::<Vec<_>>()
    );
}

fn fixture(prefix: &str) -> (crate::DocumentStream, DomHost) {
    let stream = crate::DocumentStream::new_scripting_enabled_parser_stream_for_testing(
        Url::parse("https://example.test/").unwrap(),
    );
    let mut host = stream.take_parser_stream_dom_host();
    host.set_mutation_observer_records_enabled(true);
    pump(&stream, &mut host, prefix);
    (stream, host)
}

#[test]
fn parser_text_buffer_commits_entities_raw_text_errors_and_nulls_once_per_input_boundary() {
    for (opening, input, appended) in [
        // A literal suffix completes tokenizer lookahead for the final entity.
        (
            "<main>",
            "x&amp;".repeat(2_048) + "x",
            "x&".repeat(2_048) + "x",
        ),
        (
            "<script type=application/json>",
            "x<".repeat(2_048) + "x",
            "x<".repeat(2_048) + "x",
        ),
        (
            "<main>",
            "x&#x110000;&bogus;x".into(),
            "x\u{fffd}&bogus;x".into(),
        ),
        ("<main>", "x\0y".into(), "xy".into()),
        ("<svg>", "x\0y".into(), "x\u{fffd}y".into()),
    ] {
        let prefix = format!("<!doctype html><html><body>{opening}seed");
        let (stream, mut host) = fixture(&prefix);
        let effects = pump(&stream, &mut host, &input);
        assert_old_texts(&effects, &["seed"]);
        let body = host.document_body_handle().unwrap();
        assert_eq!(host.text_content(body), Some("seed".to_owned() + &appended));
        // Flushing at a return boundary must not leave writes to be replayed
        // through the next pump's scoped runtime callbacks.
        assert!(old_texts(&pump(&stream, &mut host, "")).is_empty());
        stream.restore_parser_stream_dom_host(host);
    }
}

#[test]
fn parser_text_buffer_keeps_non_character_token_and_input_boundaries() {
    let (stream, mut host) = fixture("<!doctype html><html><body><main>seed");
    let effects = pump(&stream, &mut host, "x&amp;</ignored>y&amp;x");
    assert_old_texts(&effects, &["seed", "seedx&"]);
    let effects = pump(&stream, &mut host, "z&amp;x");
    assert_old_texts(&effects, &["seedx&y&x"]);
    let body = host.document_body_handle().unwrap();
    assert_eq!(host.text_content(body).as_deref(), Some("seedx&y&xz&x"));
    stream.restore_parser_stream_dom_host(host);
}

#[test]
fn parser_text_buffer_flushes_before_structural_mutations() {
    let (stream, mut host) = fixture("<!doctype html><html><body><main>seed");
    let effects = pump(
        &stream,
        &mut host,
        "a&amp;<span>b&amp;c</span><!--gap-->d&amp;x",
    );
    assert_old_texts(&effects, &["seed"]);
    let records = effects.observer_records().records();
    assert!(matches!(
        records[0].kind(),
        DomMutationRecordKind::CharacterData { .. }
    ));
    assert!(matches!(
        records[1].kind(),
        DomMutationRecordKind::ChildList(_)
    ));
    let body = host.document_body_handle().unwrap();
    let main = host.child_handles(body).next().unwrap();
    let children = host.child_handles(main).collect::<Vec<_>>();
    assert_eq!(children.len(), 4);
    assert_eq!(host.text_content(children[0]).as_deref(), Some("seeda&"));
    assert_eq!(host.text_content(children[1]).as_deref(), Some("b&c"));
    assert_eq!(host.text_content(children[3]).as_deref(), Some("d&x"));
    stream.restore_parser_stream_dom_host(host);
}

#[test]
fn parser_text_buffer_batches_foster_parented_text_with_attached_and_detached_tables() {
    for detach in [false, true] {
        let (stream, mut host) = fixture("<!doctype html><html><body>seed<table>");
        let body = host.document_body_handle().unwrap();
        let table = host.child_handles(body).last().unwrap();
        if detach {
            host.remove_child_effects(body, table);
        }
        let effects = pump(&stream, &mut host, &("x&amp;".repeat(2_048) + "</table>"));
        assert_old_texts(&effects, &["seed"]);
        assert_eq!(
            host.text_content(body),
            Some("seed".to_owned() + &"x&".repeat(2_048))
        );
        assert_eq!(host.child_handles(body).count(), if detach { 1 } else { 2 });
        stream.restore_parser_stream_dom_host(host);
    }
}

#[test]
fn parser_text_buffer_does_not_call_observers_with_uncommitted_text() {
    for (prefix, input, expected) in [
        (
            "<!doctype html><html><body><main>seed",
            "x&amp;".repeat(128) + "x",
            "seed".to_owned() + &"x&".repeat(128) + "x",
        ),
        // Implicit body construction during a character token can itself
        // require a callback. Its text must be committed before that callback.
        ("", "seed".to_owned(), "seed".to_owned()),
    ] {
        let (stream, mut host) = fixture(prefix);
        let mut effects = DomMutationEffects::default();
        let mut observed = Vec::new();
        let mut consumer = TestMutationEffectCollector {
            host: &mut host,
            effects: &mut effects,
            panic_on_mutation: false,
            finish_texts: Some(&mut observed),
        };
        stream.pump_parser_step_with_runtime_dom_consumer_without_element_creation(
            &input,
            &mut consumer,
        );
        assert_eq!(
            observed.len(),
            1,
            "only committed DOM work may invoke the owner"
        );
        assert_eq!(observed[0], expected);
        stream.restore_parser_stream_dom_host(host);
    }
}
