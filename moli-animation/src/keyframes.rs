/// Resolve each missing run between its adjacent explicit offsets. Offsets are
/// not sorted: an out-of-order declaration is invalid rather than reordered.
pub fn resolve_keyframe_offsets(offsets: &[Option<f64>]) -> Option<Vec<f64>> {
    let mut previous = 0.0;
    for value in offsets.iter().flatten() {
        if !value.is_finite() || !(0.0..=1.0).contains(value) || *value < previous {
            return None;
        }
        previous = *value;
    }
    if offsets.is_empty() {
        return Some(Vec::new());
    }
    let mut result = offsets.to_vec();
    let last = result.len() - 1;
    if result[last].is_none() {
        result[last] = Some(1.0);
    }
    if result[0].is_none() {
        result[0] = Some(0.0);
    }
    let mut start = 0;
    while start < last {
        let end = (start + 1..=last).find(|i| result[*i].is_some())?;
        let from = result[start]?;
        let to = result[end]?;
        for (index, value) in result.iter_mut().enumerate().take(end).skip(start + 1) {
            *value = Some(from + (to - from) * (index - start) as f64 / (end - start) as f64);
        }
        start = end;
    }
    result.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_runs_use_adjacent_explicit_offsets() {
        assert_eq!(
            resolve_keyframe_offsets(&[Some(0.0), Some(0.6), None, Some(1.0)]),
            Some(vec![0.0, 0.6, 0.8, 1.0])
        );
        let values =
            resolve_keyframe_offsets(&[None, Some(0.2), None, None, Some(0.8), None]).unwrap();
        assert!(
            values
                .iter()
                .zip([0.0, 0.2, 0.4, 0.6, 0.8, 1.0])
                .all(|(a, b)| (a - b).abs() < 1e-15)
        );
        assert_eq!(resolve_keyframe_offsets(&[None]), Some(vec![1.0]));
    }
    #[test]
    fn explicit_order_and_range_are_validated_without_sorting() {
        assert!(resolve_keyframe_offsets(&[Some(0.6), Some(0.2)]).is_none());
        assert!(resolve_keyframe_offsets(&[Some(f64::NAN)]).is_none());
        assert!(resolve_keyframe_offsets(&[Some(-0.1)]).is_none());
        assert_eq!(
            resolve_keyframe_offsets(&[Some(0.5), Some(0.5)]),
            Some(vec![0.5, 0.5])
        );
    }
}
