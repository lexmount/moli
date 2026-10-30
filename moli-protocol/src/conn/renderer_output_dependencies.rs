use moli_core::RendererOutputFence;

/// Exact renderer FIFO prefixes required before one protocol result is exposed.
///
/// Browser-level commands can compose independent Page/Worker owners. Each
/// stream retains its own lifetime lease and latest cursor; sequence numbers
/// from different streams are never compared. The usual single-owner case
/// does not allocate.
#[derive(Clone, Debug, Default)]
pub struct RendererOutputDependencies {
    first: Option<RendererOutputFence>,
    additional: Vec<RendererOutputFence>,
    explicitly_composed: bool,
}

impl RendererOutputDependencies {
    pub fn is_empty(&self) -> bool {
        self.first.is_none()
    }

    pub fn iter(&self) -> impl Iterator<Item = &RendererOutputFence> {
        self.first.iter().chain(&self.additional)
    }

    pub fn add(&mut self, fence: RendererOutputFence) {
        self.explicitly_composed = true;
        let Some(first) = self.first.as_mut() else {
            self.first = Some(fence);
            return;
        };
        let stream = fence.cursor().stream();
        if let Some(tail) = std::iter::once(first)
            .chain(&mut self.additional)
            .find(|tail| tail.cursor().stream() == stream)
        {
            *tail = tail.clone().latest_in_same_stream(fence);
        } else {
            self.additional.push(fence);
        }
    }

    /// Preserve the declaring command's ownership contract across plan and
    /// workflow handoffs. Ordinary single-owner plans must not silently gain
    /// permission to join unrelated streams merely because they are combined.
    pub fn merge(&mut self, other: Self) {
        self.explicitly_composed |= other.explicitly_composed;
        for fence in other {
            if self.explicitly_composed {
                self.add(fence);
            } else {
                self.add_same_stream(fence);
            }
        }
    }

    /// Ordinary renderer-owner operations still join exactly one source FIFO.
    /// Only explicit composition through `add`/`extend` permits other streams.
    pub fn add_same_stream(&mut self, fence: RendererOutputFence) {
        fence.merge_into_same_stream_tail(self.single_stream_mut());
    }

    pub(crate) fn single_stream_mut(&mut self) -> &mut Option<RendererOutputFence> {
        assert!(
            self.additional.is_empty(),
            "a single renderer owner cannot consume composite output dependencies"
        );
        &mut self.first
    }

    #[track_caller]
    pub fn into_single_stream(self) -> Option<RendererOutputFence> {
        assert!(
            self.additional.is_empty(),
            "composite output dependencies require consuming every renderer stream"
        );
        self.first
    }
}

impl From<Option<RendererOutputFence>> for RendererOutputDependencies {
    fn from(first: Option<RendererOutputFence>) -> Self {
        Self {
            first,
            additional: Vec::new(),
            explicitly_composed: false,
        }
    }
}

impl Extend<RendererOutputFence> for RendererOutputDependencies {
    fn extend<T: IntoIterator<Item = RendererOutputFence>>(&mut self, fences: T) {
        for fence in fences {
            self.add(fence);
        }
    }
}

impl IntoIterator for RendererOutputDependencies {
    type Item = RendererOutputFence;
    type IntoIter = std::iter::Chain<
        std::option::IntoIter<RendererOutputFence>,
        std::vec::IntoIter<RendererOutputFence>,
    >;

    fn into_iter(self) -> Self::IntoIter {
        self.first.into_iter().chain(self.additional)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moli_core::{PageId, RendererOutputCursor, RendererOutputStreamIdentity};

    #[test]
    fn dependencies_keep_the_latest_cursor_in_each_exact_stream() {
        let first =
            RendererOutputStreamIdentity::new_page_for_protocol_test(PageId::new_for_testing(71));
        let second =
            RendererOutputStreamIdentity::new_page_for_protocol_test(PageId::new_for_testing(72));
        // Reusing a Page ID does not reuse its renderer agent or stream epoch.
        let replacement =
            RendererOutputStreamIdentity::new_page_for_protocol_test(PageId::new_for_testing(71));
        let mut dependencies = RendererOutputDependencies::default();
        assert!(dependencies.is_empty());
        for (stream, sequence) in [
            (first, 3),
            (second, 1),
            (first, 2),
            (first, 5),
            (replacement, 1),
        ] {
            dependencies.add(RendererOutputFence::new_for_test(
                RendererOutputCursor::new_for_test(stream, sequence),
            ));
        }
        assert_eq!(
            dependencies
                .iter()
                .map(|fence| fence.cursor())
                .collect::<Vec<_>>(),
            [(first, 5), (second, 1), (replacement, 1)]
                .map(|(stream, sequence)| RendererOutputCursor::new_for_test(stream, sequence))
        );
    }

    #[test]
    #[should_panic(expected = "one command response cannot join unrelated renderer output streams")]
    fn ordinary_owner_dependencies_keep_the_single_stream_constraint() {
        let mut dependencies = RendererOutputDependencies::default();
        for page in [71, 72] {
            let stream = RendererOutputStreamIdentity::new_page_for_protocol_test(
                PageId::new_for_testing(page),
            );
            dependencies.add_same_stream(RendererOutputFence::new_for_test(
                RendererOutputCursor::new_for_test(stream, 1),
            ));
        }
    }

    #[test]
    #[should_panic(expected = "one command response cannot join unrelated renderer output streams")]
    fn merging_ordinary_plans_does_not_enable_cross_stream_composition() {
        let mut dependencies = RendererOutputDependencies::default();
        for page in [71, 72] {
            let stream = RendererOutputStreamIdentity::new_page_for_protocol_test(
                PageId::new_for_testing(page),
            );
            dependencies.merge(
                Some(RendererOutputFence::new_for_test(
                    RendererOutputCursor::new_for_test(stream, 1),
                ))
                .into(),
            );
        }
    }

    #[test]
    fn explicit_composition_survives_plan_handoffs_with_one_stream() {
        for composed_first in [false, true] {
            let first = RendererOutputStreamIdentity::new_page_for_protocol_test(
                PageId::new_for_testing(71),
            );
            let second = RendererOutputStreamIdentity::new_page_for_protocol_test(
                PageId::new_for_testing(72),
            );
            let mut composed = RendererOutputDependencies::default();
            composed.add(RendererOutputFence::new_for_test(
                RendererOutputCursor::new_for_test(first, 1),
            ));
            let ordinary = Some(RendererOutputFence::new_for_test(
                RendererOutputCursor::new_for_test(second, 1),
            ))
            .into();
            let mut left = composed;
            let mut right = ordinary;
            if !composed_first {
                std::mem::swap(&mut left, &mut right);
            }
            left.merge(right);
            assert_eq!(left.iter().count(), 2);
        }
    }
}
