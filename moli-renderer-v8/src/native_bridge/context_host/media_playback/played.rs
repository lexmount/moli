use super::{DomHandle, JsContextHost};

/// Timeline intervals crossed by normal forward playback, independent of seeks.
#[derive(Default)]
pub(super) struct MediaPlayedRanges {
    ranges: Vec<(f64, f64)>,
}

impl MediaPlayedRanges {
    pub(super) fn record(&mut self, mut start: f64, mut end: f64) {
        if !start.is_finite() || !end.is_finite() || start < 0.0 || end <= start {
            return;
        }
        let first = self.ranges.partition_point(|&(_, end)| end < start);
        let mut last = first;
        while let Some(&(range_start, range_end)) = self.ranges.get(last)
            && range_start <= end
        {
            start = start.min(range_start);
            end = end.max(range_end);
            last += 1;
        }
        self.ranges.splice(first..last, [(start, end)]);
    }

    pub(super) fn snapshot(&self) -> Vec<(f64, f64)> {
        self.ranges.clone()
    }

    pub(super) fn clear(&mut self) {
        self.ranges.clear();
    }
}

impl JsContextHost {
    pub(crate) fn media_played_ranges(&self, media: DomHandle) -> Vec<(f64, f64)> {
        let Some(document) = self
            .dom_host()
            .node(media)
            .and_then(crate::dom::native::Node::owner_document)
        else {
            return Vec::new();
        };
        self.media_playback
            .elements
            .get(&media)
            .filter(|state| state.document == document)
            .map_or_else(Vec::new, |state| state.played.snapshot())
    }
}

#[cfg(test)]
mod tests {
    use super::MediaPlayedRanges;

    #[test]
    fn played_ranges_normalize_replays_touching_intervals_and_gaps() {
        let mut played = MediaPlayedRanges::default();
        for (start, end) in [(4.0, 5.0), (1.0, 2.0), (3.0, 4.0), (1.5, 1.75)] {
            played.record(start, end);
        }
        assert_eq!(played.snapshot(), [(1.0, 2.0), (3.0, 5.0)]);
        played.record(2.0, 3.0);
        assert_eq!(played.snapshot(), [(1.0, 5.0)]);
        played.record(0.0, 6.0);
        assert_eq!(played.snapshot(), [(0.0, 6.0)]);
    }

    #[test]
    fn played_ranges_ignore_non_forward_advances_and_keep_old_snapshots() {
        let mut played = MediaPlayedRanges::default();
        for (start, end) in [
            (1.0, 1.0),
            (2.0, 1.0),
            (-1.0, 1.0),
            (0.0, f64::NAN),
            (0.0, f64::INFINITY),
        ] {
            played.record(start, end);
        }
        assert!(played.snapshot().is_empty());
        played.record(1.0, 2.0);
        let snapshot = played.snapshot();
        played.clear();
        played.record(3.0, 4.0);
        assert_eq!(snapshot, [(1.0, 2.0)]);
        assert_eq!(played.snapshot(), [(3.0, 4.0)]);
    }
}
