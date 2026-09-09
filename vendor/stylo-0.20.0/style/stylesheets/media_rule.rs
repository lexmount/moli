/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! An [`@media`][media] rule.
//!
//! [media]: https://drafts.csswg.org/css-conditional/#at-ruledef-media

use crate::derives::*;
use crate::media_queries::MediaList;
use crate::selector_map::{PrecomputedHashMap, PrecomputedHashSet};
use crate::shared_lock::{DeepCloneWithLock, Locked};
use crate::shared_lock::{SharedRwLock, SharedRwLockReadGuard, ToCssWithGuard};
use crate::stylesheets::CssRules;
use crate::values::{computed, DashedIdent};
use crate::Atom;
use cssparser::Parser;
use cssparser::SourceLocation;
#[cfg(feature = "gecko")]
use malloc_size_of::{MallocSizeOfOps, MallocUnconditionalShallowSizeOf};
use selectors::kleene_value::KleeneValue;
use servo_arc::Arc;
use std::fmt::{self, Write};
use style_traits::{CssStringWriter, CssWriter, ParseError, ToCss};

/// An [`@media`][media] rule.
///
/// [media]: https://drafts.csswg.org/css-conditional/#at-ruledef-media
#[derive(Debug, ToShmem)]
pub struct MediaRule {
    /// The list of media queries used by this media rule.
    pub media_queries: Arc<Locked<MediaList>>,
    /// The nested rules to this media rule.
    pub rules: Arc<Locked<CssRules>>,
    /// The source position where this media rule was found.
    pub source_location: SourceLocation,
}

impl MediaRule {
    /// Measure heap usage.
    #[cfg(feature = "gecko")]
    pub fn size_of(&self, guard: &SharedRwLockReadGuard, ops: &mut MallocSizeOfOps) -> usize {
        // Measurement of other fields may be added later.
        self.rules.unconditional_shallow_size_of(ops)
            + self.rules.read_with(guard).size_of(guard, ops)
    }
}

impl ToCssWithGuard for MediaRule {
    // Serialization of MediaRule is not specced.
    // https://drafts.csswg.org/cssom/#serialize-a-css-rule CSSMediaRule
    fn to_css(&self, guard: &SharedRwLockReadGuard, dest: &mut CssStringWriter) -> fmt::Result {
        dest.write_str("@media ")?;
        self.media_queries
            .read_with(guard)
            .to_css(&mut CssWriter::new(dest))?;
        self.rules.read_with(guard).to_css_block(guard, dest)
    }
}

impl DeepCloneWithLock for MediaRule {
    fn deep_clone_with_lock(&self, lock: &SharedRwLock, guard: &SharedRwLockReadGuard) -> Self {
        let media_queries = self.media_queries.read_with(guard);
        let rules = self.rules.read_with(guard);
        MediaRule {
            media_queries: Arc::new(lock.wrap(media_queries.clone())),
            rules: Arc::new(lock.wrap(rules.deep_clone_with_lock(lock, guard))),
            source_location: self.source_location.clone(),
        }
    }
}

/// The condition associated to a custom-media query.
#[derive(Debug, ToShmem, Clone, MallocSizeOf)]
pub enum CustomMediaCondition {
    /// Unconditionally true.
    True,
    /// Unconditionally false.
    False,
    /// A MediaList.
    MediaList(#[ignore_malloc_size_of = "Arc"] Arc<Locked<MediaList>>),
}

impl CustomMediaCondition {
    /// Parses the possible keywords for this condition.
    pub(crate) fn parse_keyword<'i>(input: &mut Parser<'i, '_>) -> Result<Self, ParseError<'i>> {
        Ok(try_match_ident_ignore_ascii_case! { input,
            "true" => Self::True,
            "false" => Self::False,
        })
    }
}

impl DeepCloneWithLock for CustomMediaCondition {
    fn deep_clone_with_lock(&self, lock: &SharedRwLock, guard: &SharedRwLockReadGuard) -> Self {
        match self {
            Self::True => Self::True,
            Self::False => Self::False,
            Self::MediaList(ref m) => {
                Self::MediaList(Arc::new(lock.wrap(m.read_with(guard).clone())))
            },
        }
    }
}

/// A `@custom-media` rule.
/// https://drafts.csswg.org/mediaqueries-5/#custom-mq
#[derive(Debug, ToShmem)]
pub struct CustomMediaRule {
    /// The name of the custom media rule.
    pub name: DashedIdent,
    /// The list of media conditions used by this media rule.
    pub condition: CustomMediaCondition,
    /// The source position where this media rule was found.
    pub source_location: SourceLocation,
}

impl DeepCloneWithLock for CustomMediaRule {
    fn deep_clone_with_lock(&self, lock: &SharedRwLock, guard: &SharedRwLockReadGuard) -> Self {
        Self {
            name: self.name.clone(),
            condition: self.condition.deep_clone_with_lock(lock, guard),
            source_location: self.source_location.clone(),
        }
    }
}

impl ToCssWithGuard for CustomMediaRule {
    // Serialization of MediaRule is not specced.
    // https://drafts.csswg.org/cssom/#serialize-a-css-rule CSSMediaRule
    fn to_css(&self, guard: &SharedRwLockReadGuard, dest: &mut CssStringWriter) -> fmt::Result {
        dest.write_str("@custom-media ")?;
        self.name.to_css(&mut CssWriter::new(dest))?;
        dest.write_char(' ')?;
        match self.condition {
            CustomMediaCondition::True => dest.write_str("true"),
            CustomMediaCondition::False => dest.write_str("false"),
            CustomMediaCondition::MediaList(ref m) => {
                m.read_with(guard).to_css(&mut CssWriter::new(dest))
            },
        }
    }
}

/// The currently effective @custom-media conditions.
pub type CustomMediaMap = PrecomputedHashMap<Atom, CustomMediaCondition>;

/// A struct that can evaluate custom media conditions.
pub struct CustomMediaEvaluator<'a> {
    map: Option<(&'a CustomMediaMap, &'a SharedRwLockReadGuard<'a>)>,
    /// Set of media queries we're currently evaluating, needed for cycle detection.
    currently_evaluating: PrecomputedHashSet<Atom>,
}

impl<'a> CustomMediaEvaluator<'a> {
    /// Construct a new custom media evaluator with the given map and guard.
    pub fn new(map: &'a CustomMediaMap, guard: &'a SharedRwLockReadGuard<'a>) -> Self {
        Self {
            map: Some((map, guard)),
            currently_evaluating: Default::default(),
        }
    }

    /// Returns an evaluator that can't evaluate custom queries.
    pub fn none() -> Self {
        Self {
            map: None,
            currently_evaluating: Default::default(),
        }
    }

    /// Evaluates a custom media query.
    pub fn matches(&mut self, ident: &DashedIdent, context: &computed::Context) -> KleeneValue {
        let Some((map, guard)) = self.map else {
            return KleeneValue::Unknown;
        };
        let Some(condition) = map.get(&ident.0) else {
            return KleeneValue::Unknown;
        };
        let media = match condition {
            CustomMediaCondition::True => return KleeneValue::True,
            CustomMediaCondition::False => return KleeneValue::False,
            CustomMediaCondition::MediaList(ref m) => m,
        };
        if !self.currently_evaluating.insert(ident.0.clone()) {
            // Found a cycle while evaluating this rule.
            return KleeneValue::False;
        }
        let result = media.read_with(guard).matches(context, self);
        self.currently_evaluating.remove(&ident.0);
        result.into()
    }
}
