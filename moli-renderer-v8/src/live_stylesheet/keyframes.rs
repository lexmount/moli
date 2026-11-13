use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
};

use moli_css_parse::CssRuleSnapshot;
use style::{
    shared_lock::{SharedRwLock, SharedRwLockReadGuard},
    stylesheets::{CssRule, Stylesheet},
};

pub(crate) const KEYFRAME_NESTING_DEPTH_LIMIT: usize = 32;

/// A revision-scoped name lookup over the authoritative native rules. Keep
/// references to keyframes only, and project snapshots lazily for queried names.
/// Reentrant animation listeners start a fresh style observation but can reuse
/// this index until a CSSOM contents mutation invalidates it.
#[derive(Debug)]
pub(crate) struct StylesheetKeyframeIndex {
    shared_lock: SharedRwLock,
    rules_by_name: HashMap<String, Vec<IndexedKeyframesRule>>,
}

#[derive(Debug)]
struct IndexedKeyframesRule {
    order: usize,
    rule: CssRule,
    snapshot: OnceLock<Arc<CssRuleSnapshot>>,
}

impl StylesheetKeyframeIndex {
    pub(crate) fn new(stylesheet: &Stylesheet) -> Self {
        #[cfg(test)]
        METRICS.with(|metrics| metrics.borrow_mut().builds += 1);

        let mut index = Self {
            shared_lock: stylesheet.shared_lock.clone(),
            rules_by_name: HashMap::new(),
        };
        let guard = stylesheet.shared_lock.read();
        let contents = stylesheet.contents.read_with(&guard);
        let mut order = 0;
        index.collect(&contents.rules.read_with(&guard).0, &guard, 0, &mut order);
        index
    }

    fn collect(
        &mut self,
        rules: &[CssRule],
        guard: &SharedRwLockReadGuard<'_>,
        depth: usize,
        order: &mut usize,
    ) {
        if depth > KEYFRAME_NESTING_DEPTH_LIMIT {
            return;
        }
        for rule in rules {
            #[cfg(test)]
            METRICS.with(|metrics| metrics.borrow_mut().rule_visits += 1);

            match rule {
                CssRule::Keyframes(keyframes) => {
                    let name = str::to_ascii_lowercase(keyframes.read_with(guard).name.as_atom());
                    self.rules_by_name
                        .entry(name)
                        .or_default()
                        .push(IndexedKeyframesRule {
                            order: *order,
                            rule: rule.clone(),
                            snapshot: OnceLock::new(),
                        });
                    *order += 1;
                }
                CssRule::Media(_) | CssRule::Supports(_) => {
                    self.collect(rule.children(guard), guard, depth + 1, order);
                }
                _ => {}
            }
        }
    }

    pub(crate) fn rule_snapshots(&self, animation_names: &[String]) -> Vec<Arc<CssRuleSnapshot>> {
        let mut matches = animation_names
            .iter()
            .filter_map(|name| self.rules_by_name.get(&name.to_ascii_lowercase()))
            .flatten()
            .collect::<Vec<_>>();
        // Preserve stylesheet order across distinct names and duplicate rules,
        // even when animation-name lists those names in a different order.
        matches.sort_unstable_by_key(|entry| entry.order);
        matches.dedup_by_key(|entry| entry.order);
        let guard = self.shared_lock.read();
        matches
            .into_iter()
            .map(|entry| {
                Arc::clone(entry.snapshot.get_or_init(|| {
                    #[cfg(test)]
                    METRICS.with(|metrics| metrics.borrow_mut().snapshots += 1);

                    Arc::new(moli_css_parse::css_rule_snapshot_from_native_with_stylo(
                        &entry.rule,
                        &guard,
                    ))
                }))
            })
            .collect()
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct KeyframeIndexMetrics {
    pub(crate) builds: usize,
    pub(crate) rule_visits: usize,
    pub(crate) snapshots: usize,
}

#[cfg(test)]
thread_local! {
    static METRICS: std::cell::RefCell<KeyframeIndexMetrics> = const {
        std::cell::RefCell::new(KeyframeIndexMetrics {
            builds: 0,
            rule_visits: 0,
            snapshots: 0,
        })
    };
}

#[cfg(test)]
pub(crate) fn reset_keyframe_index_metrics_for_test() {
    METRICS.with(|metrics| *metrics.borrow_mut() = KeyframeIndexMetrics::default());
}

#[cfg(test)]
pub(crate) fn keyframe_index_metrics_for_test() -> KeyframeIndexMetrics {
    METRICS.with(|metrics| *metrics.borrow())
}
