use moli_animation::{AnimationState, KeyframeValue, PropertyKeyframes};
use std::time::Instant;

fn effect(offsets: &[f64]) -> AnimationState {
    let mut state = AnimationState::new(Instant::now());
    state.frames = vec![PropertyKeyframes {
        property: "opacity".to_owned(),
        values: offsets
            .iter()
            .enumerate()
            .map(|(index, offset)| KeyframeValue::new(*offset, index.to_string()))
            .collect(),
    }];
    state
}

#[test]
fn exact_offset_uses_the_following_interval_and_its_easing() {
    let mut state = effect(&[0.0, 0.5, 1.0]);
    state.frames[0].values[1].easing = Some("steps(1,start)".into());
    let sample = state.sample_interval("opacity", 0.5).unwrap();
    assert_eq!(
        (sample.from.as_deref(), sample.to.as_deref()),
        (Some("1"), Some("2"))
    );
    assert_eq!(sample.progress, 0.0);
    assert_eq!(sample.easing.as_deref(), Some("steps(1,start)"));
}

#[test]
fn duplicate_endpoint_offsets_keep_the_outermost_value() {
    let state = effect(&[0.0, 0.0, 1.0, 1.0]);
    let before = state.sample_interval("opacity", -0.1).unwrap();
    assert_eq!(
        (before.from.as_deref(), before.to.as_deref()),
        (Some("0"), Some("0"))
    );
    for progress in [1.0, 1.1] {
        let after = state.sample_interval("opacity", progress).unwrap();
        assert_eq!(
            (after.from.as_deref(), after.to.as_deref()),
            (Some("3"), Some("3"))
        );
    }
}

#[test]
fn neutral_endpoints_survive_interval_sampling() {
    let now = std::time::Instant::now();
    let mut state = moli_animation::AnimationState::css(now);
    state.frames = vec![moli_animation::PropertyKeyframes {
        property: "opacity".into(),
        values: vec![
            moli_animation::KeyframeValue::underlying(0.0),
            moli_animation::KeyframeValue::new(1.0, "1".into()),
        ],
    }];
    let sample = state.sample_interval("opacity", 0.5).unwrap();
    assert_eq!(sample.from, None);
    assert_eq!(sample.to.as_deref(), Some("1"));
    assert_eq!(sample.progress, 0.5);
}

#[test]
fn neutral_admission_preserves_authored_values_and_is_idempotent() {
    let mut frames = PropertyKeyframes {
        property: "width".into(),
        values: vec![KeyframeValue::new(1.0, "3em".into())],
    };
    frames.ensure_neutral_endpoints();
    frames.ensure_neutral_endpoints();
    assert_eq!(frames.values.len(), 2);
    assert_eq!(frames.values[0].value, None);
    assert_eq!(frames.values[1].value.as_deref(), Some("3em"));
}
