//! Opt-in test instrumentation; absent from default production builds.
use std::cell::Cell;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DomAccessCounts {
    pub clones: usize,
    pub mutable_node_accesses: usize,
    pub mutation_api_entries: usize,
}

thread_local! {
    static ACTIVE: Cell<Option<DomAccessCounts>> = const { Cell::new(None) };
}

pub struct DomAccessScope {
    finished: bool,
}
impl DomAccessScope {
    pub fn begin() -> Self {
        ACTIVE.with(|active| {
            assert!(active.get().is_none(), "DOM access scopes cannot nest");
            active.set(Some(DomAccessCounts::default()));
        });
        Self { finished: false }
    }
    pub fn finish(mut self) -> DomAccessCounts {
        self.finished = true;
        ACTIVE.with(|active| active.take().expect("active DOM access scope"))
    }
}
impl Drop for DomAccessScope {
    fn drop(&mut self) {
        if !self.finished {
            ACTIVE.with(|active| active.set(None));
        }
    }
}

pub(super) enum Access {
    Clone,
    MutableNode,
    Mutation,
}
pub(super) fn record(access: Access) {
    ACTIVE.with(|active| {
        if let Some(mut count) = active.get() {
            match access {
                Access::Clone => count.clones += 1,
                Access::MutableNode => count.mutable_node_accesses += 1,
                Access::Mutation => count.mutation_api_entries += 1,
            }
            active.set(Some(count));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::NativeDom;
    #[test]
    fn access_probe_observes_copy_and_mutation_attempts_and_resets() {
        let mut dom = NativeDom::new_html("https://probe.test/".parse().unwrap());
        let node = dom.create_element("div");
        let scope = DomAccessScope::begin();
        let _copy = dom.clone();
        assert!(dom.set_attribute(node, "x", "y"));
        let counts = scope.finish();
        assert_eq!(counts.clones, 1);
        assert!(counts.mutable_node_accesses > 0);
        assert!(counts.mutation_api_entries > 0);
        assert_eq!(DomAccessScope::begin().finish(), DomAccessCounts::default());
    }
}
