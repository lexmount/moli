/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! The `@appearance-base` rule, for UA stylesheet appearance-dependent styles.

use crate::derives::*;
use crate::shared_lock::{DeepCloneWithLock, Locked};
use crate::shared_lock::{SharedRwLock, SharedRwLockReadGuard, ToCssWithGuard};
use crate::stylesheets::CssRules;
use cssparser::SourceLocation;
#[cfg(feature = "gecko")]
use malloc_size_of::{MallocSizeOfOps, MallocUnconditionalShallowSizeOf};
use servo_arc::Arc;
use std::fmt::{self, Write};
use style_traits::CssStringWriter;

/// An `@appearance-base` rule.
#[derive(Debug, ToShmem)]
pub struct AppearanceBaseRule {
    /// The nested rules inside this @appearance-base rule.
    pub rules: Arc<Locked<CssRules>>,
    /// The source position where this rule was found.
    pub source_location: SourceLocation,
}

impl AppearanceBaseRule {
    /// Measure heap usage.
    #[cfg(feature = "gecko")]
    pub fn size_of(&self, guard: &SharedRwLockReadGuard, ops: &mut MallocSizeOfOps) -> usize {
        self.rules.unconditional_shallow_size_of(ops)
            + self.rules.read_with(guard).size_of(guard, ops)
    }
}

impl ToCssWithGuard for AppearanceBaseRule {
    fn to_css(&self, guard: &SharedRwLockReadGuard, dest: &mut CssStringWriter) -> fmt::Result {
        dest.write_str("@appearance-base")?;
        self.rules.read_with(guard).to_css_block(guard, dest)
    }
}

impl DeepCloneWithLock for AppearanceBaseRule {
    fn deep_clone_with_lock(&self, lock: &SharedRwLock, guard: &SharedRwLockReadGuard) -> Self {
        let rules = self.rules.read_with(guard);
        AppearanceBaseRule {
            rules: Arc::new(lock.wrap(rules.deep_clone_with_lock(lock, guard))),
            source_location: self.source_location.clone(),
        }
    }
}
