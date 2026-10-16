use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DOMAIN: AtomicU64 = AtomicU64::new(1);

/// Sparse-by-storage-chunk invalidation, allocated only by retained layout.
/// Revisions advance on mutable access rather than selected mutation APIs,
/// so parsing, attributes, text, tree links and native control state cannot
/// silently bypass invalidation. Extra invalidations are harmless.
#[derive(Debug)]
pub(super) struct NodeMutationTracking {
    domain: u64,
    sequence: Option<u64>,
    chunks: Vec<u64>,
}

impl NodeMutationTracking {
    pub(super) fn new() -> Option<Self> {
        let domain = NEXT_DOMAIN
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .ok()?;
        Some(Self {
            domain,
            sequence: Some(0),
            chunks: Vec::new(),
        })
    }

    pub(super) fn changed(&mut self, chunk: usize) {
        self.sequence = self.sequence.and_then(|value| value.checked_add(1));
        if let Some(sequence) = self.sequence {
            self.chunks.resize(self.chunks.len().max(chunk + 1), 0);
            self.chunks[chunk] = sequence;
        }
    }

    pub(super) fn revision(&self, chunk: usize) -> Option<[u64; 2]> {
        self.sequence?;
        Some([self.domain, self.chunks.get(chunk).copied().unwrap_or(0)])
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::NodeMutationTracking;

    #[test]
    fn tracked_chunks_invalidate_mutations_and_cow_views_have_separate_domains() {
        let mut dom = NativeDom::new(url::Url::parse("https://example.test/").unwrap());
        let root = dom.document_node_id();
        assert_eq!(dom.node_mutation_revision(root), None);
        dom.enable_node_mutation_tracking();
        let before = dom.node_mutation_revision(root).unwrap();
        let mut copy = dom.clone();
        assert_eq!(copy.node_mutation_revision(root), None);
        copy.enable_node_mutation_tracking();
        assert_ne!(copy.node_mutation_revision(root).unwrap()[0], before[0]);
        dom.node_mut(root).unwrap();
        assert_ne!(dom.node_mutation_revision(root).unwrap(), before);
        let copy_before = copy.node_mutation_revision(root).unwrap();
        copy.node_mut(root).unwrap();
        assert_ne!(copy.node_mutation_revision(root).unwrap(), copy_before);
        assert_ne!(
            copy.node_mutation_revision(root),
            dom.node_mutation_revision(root)
        );
    }

    #[test]
    fn unrelated_chunks_and_exhaustion_are_conservative() {
        let mut tracking = NodeMutationTracking::new().unwrap();
        let before = tracking.revision(1);
        tracking.changed(0);
        assert_eq!(tracking.revision(1), before);
        tracking.changed(1);
        assert_ne!(tracking.revision(1), before);
        tracking.sequence = Some(u64::MAX);
        tracking.changed(0);
        assert_eq!(tracking.revision(0), None);
        assert_eq!(tracking.revision(1), None);
    }
}
