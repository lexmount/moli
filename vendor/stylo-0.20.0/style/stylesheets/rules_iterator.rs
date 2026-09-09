/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! An iterator over a list of rules.

use crate::context::QuirksMode;
use crate::device::Device;
use crate::shared_lock::SharedRwLockReadGuard;
use crate::stylesheets::{
    CssRule, CssRuleRef, CustomMediaEvaluator, CustomMediaMap, DocumentRule, ImportRule, MediaRule,
    SupportsRule,
};
use smallvec::SmallVec;
use std::ops::Deref;
use std::slice;

/// An iterator over a list of rules.
pub struct RulesIterator<'a, 'b, C, CMM>
where
    'b: 'a,
    C: NestedRuleIterationCondition + 'static,
    CMM: Deref<Target = CustomMediaMap>,
{
    device: &'a Device,
    quirks_mode: QuirksMode,
    custom_media: CMM,
    guard: &'a SharedRwLockReadGuard<'b>,
    stack: SmallVec<[slice::Iter<'a, CssRule>; 3]>,
    last_rule_had_children: bool,
    _phantom: ::std::marker::PhantomData<C>,
}

impl<'a, 'b, C, CMM> RulesIterator<'a, 'b, C, CMM>
where
    'b: 'a,
    C: NestedRuleIterationCondition + 'static,
    CMM: Deref<Target = CustomMediaMap>,
{
    /// Returns the custom media map passed at construction.
    pub fn custom_media(&mut self) -> &mut CMM {
        &mut self.custom_media
    }

    /// Creates a new `RulesIterator` to iterate over `rules`.
    pub fn new(
        device: &'a Device,
        quirks_mode: QuirksMode,
        custom_media: CMM,
        guard: &'a SharedRwLockReadGuard<'b>,
        rules: slice::Iter<'a, CssRule>,
    ) -> Self {
        let mut stack = SmallVec::new();
        stack.push(rules);
        Self {
            device,
            quirks_mode,
            custom_media,
            guard,
            stack,
            last_rule_had_children: false,
            _phantom: ::std::marker::PhantomData,
        }
    }

    /// Skips all the remaining children of the last nested rule processed.
    pub fn skip_children(&mut self) {
        if self.last_rule_had_children {
            self.stack.pop();
            self.last_rule_had_children = false;
        }
    }

    /// Returns the children of `rule`, and whether `rule` is effective.
    pub fn children(
        rule: &'a CssRule,
        device: &'a Device,
        quirks_mode: QuirksMode,
        custom_media_map: &CustomMediaMap,
        guard: &'a SharedRwLockReadGuard<'_>,
        effective: &mut bool,
    ) -> &'a [CssRule] {
        *effective = true;
        match *rule {
            CssRule::Namespace(_)
            | CssRule::FontFace(_)
            | CssRule::CounterStyle(_)
            | CssRule::CustomMedia(_)
            | CssRule::Keyframes(_)
            | CssRule::Margin(_)
            | CssRule::Property(_)
            | CssRule::LayerStatement(_)
            | CssRule::FontFeatureValues(_)
            | CssRule::FontPaletteValues(_)
            | CssRule::NestedDeclarations(_)
            | CssRule::PositionTry(_)
            | CssRule::ViewTransition(_) => &[],
            CssRule::Page(ref page_rule) => {
                let page_rule = page_rule.read_with(guard);
                let rules = page_rule.rules.read_with(guard);
                rules.0.as_slice()
            },
            CssRule::Style(ref style_rule) => {
                let style_rule = style_rule.read_with(guard);
                match style_rule.rules.as_ref() {
                    Some(r) => r.read_with(guard).0.as_slice(),
                    None => &[],
                }
            },
            CssRule::Import(ref import_rule) => {
                let import_rule = import_rule.read_with(guard);
                if !C::process_import(guard, device, quirks_mode, custom_media_map, import_rule) {
                    *effective = false;
                    return &[];
                }
                import_rule.stylesheet.rules(guard)
            },
            CssRule::Document(ref doc_rule) => {
                if !C::process_document(guard, device, quirks_mode, doc_rule) {
                    *effective = false;
                    return &[];
                }
                doc_rule.rules.read_with(guard).0.as_slice()
            },
            CssRule::Container(ref container_rule) => {
                container_rule.rules.read_with(guard).0.as_slice()
            },
            CssRule::Media(ref media_rule) => {
                if !C::process_media(guard, device, quirks_mode, custom_media_map, media_rule) {
                    *effective = false;
                    return &[];
                }
                media_rule.rules.read_with(guard).0.as_slice()
            },
            CssRule::Supports(ref supports_rule) => {
                if !C::process_supports(guard, device, quirks_mode, supports_rule) {
                    *effective = false;
                    return &[];
                }
                supports_rule.rules.read_with(guard).0.as_slice()
            },
            CssRule::LayerBlock(ref layer_rule) => layer_rule.rules.read_with(guard).0.as_slice(),
            CssRule::Scope(ref rule) => rule.rules.read_with(guard).0.as_slice(),
            CssRule::StartingStyle(ref rule) => rule.rules.read_with(guard).0.as_slice(),
            CssRule::AppearanceBase(ref rule) => rule.rules.read_with(guard).0.as_slice(),
        }
    }
}

impl<'a, 'b, C, CMM> Iterator for RulesIterator<'a, 'b, C, CMM>
where
    'b: 'a,
    C: NestedRuleIterationCondition + 'static,
    CMM: Deref<Target = CustomMediaMap>,
{
    type Item = &'a CssRule;

    fn next(&mut self) -> Option<Self::Item> {
        self.last_rule_had_children = false;
        while !self.stack.is_empty() {
            let rule = {
                let nested_iter = self.stack.last_mut().unwrap();
                match nested_iter.next() {
                    Some(r) => r,
                    None => {
                        self.stack.pop();
                        continue;
                    },
                }
            };

            let mut effective = true;
            let children = Self::children(
                rule,
                self.device,
                self.quirks_mode,
                &self.custom_media,
                self.guard,
                &mut effective,
            );
            if !effective {
                continue;
            }
            if !children.is_empty() {
                debug_assert_eq!(
                    rule.children(self.guard).len(),
                    children.len(),
                    "Should agree with CssRule::children if effective"
                );
                self.last_rule_had_children = true;
                self.stack.push(children.iter());
            }
            return Some(rule);
        }

        None
    }
}

/// RulesIterator.
pub trait NestedRuleIterationCondition {
    /// Whether we should process the nested rules in a given `@import` rule.
    fn process_import(
        guard: &SharedRwLockReadGuard,
        device: &Device,
        quirks_mode: QuirksMode,
        custom_media_map: &CustomMediaMap,
        rule: &ImportRule,
    ) -> bool;

    /// Whether we should process the nested rules in a given `@media` rule.
    fn process_media(
        guard: &SharedRwLockReadGuard,
        device: &Device,
        quirks_mode: QuirksMode,
        custom_media_map: &CustomMediaMap,
        rule: &MediaRule,
    ) -> bool;

    /// Whether we should process the nested rules in a given `@-moz-document`
    /// rule.
    fn process_document(
        guard: &SharedRwLockReadGuard,
        device: &Device,
        quirks_mode: QuirksMode,
        rule: &DocumentRule,
    ) -> bool;

    /// Whether we should process the nested rules in a given `@supports` rule.
    fn process_supports(
        guard: &SharedRwLockReadGuard,
        device: &Device,
        quirks_mode: QuirksMode,
        rule: &SupportsRule,
    ) -> bool;
}

/// A struct that represents the condition that a rule applies to the document.
pub struct EffectiveRules;

impl EffectiveRules {
    /// Returns whether a given rule is effective.
    pub fn is_effective(
        guard: &SharedRwLockReadGuard,
        device: &Device,
        quirks_mode: QuirksMode,
        custom_media_map: &CustomMediaMap,
        rule: &CssRuleRef,
    ) -> bool {
        match *rule {
            CssRuleRef::Import(import_rule) => {
                let import_rule = import_rule.read_with(guard);
                Self::process_import(guard, device, quirks_mode, custom_media_map, import_rule)
            },
            CssRuleRef::Document(doc_rule) => {
                Self::process_document(guard, device, quirks_mode, doc_rule)
            },
            CssRuleRef::Media(media_rule) => {
                Self::process_media(guard, device, quirks_mode, custom_media_map, media_rule)
            },
            CssRuleRef::Supports(supports_rule) => {
                Self::process_supports(guard, device, quirks_mode, supports_rule)
            },
            _ => true,
        }
    }
}

impl NestedRuleIterationCondition for EffectiveRules {
    fn process_import(
        guard: &SharedRwLockReadGuard,
        device: &Device,
        quirks_mode: QuirksMode,
        custom_media_map: &CustomMediaMap,
        rule: &ImportRule,
    ) -> bool {
        match rule.stylesheet.media(guard) {
            Some(m) => m.evaluate(
                device,
                quirks_mode,
                &mut CustomMediaEvaluator::new(custom_media_map, guard),
            ),
            None => true,
        }
    }

    fn process_media(
        guard: &SharedRwLockReadGuard,
        device: &Device,
        quirks_mode: QuirksMode,
        custom_media_map: &CustomMediaMap,
        rule: &MediaRule,
    ) -> bool {
        rule.media_queries.read_with(guard).evaluate(
            device,
            quirks_mode,
            &mut CustomMediaEvaluator::new(custom_media_map, guard),
        )
    }

    fn process_document(
        _: &SharedRwLockReadGuard,
        device: &Device,
        _: QuirksMode,
        rule: &DocumentRule,
    ) -> bool {
        rule.condition.evaluate(device)
    }

    fn process_supports(
        _: &SharedRwLockReadGuard,
        _: &Device,
        _: QuirksMode,
        rule: &SupportsRule,
    ) -> bool {
        rule.enabled
    }
}

/// A filter that processes all the rules in a rule list.
pub struct AllRules;

impl NestedRuleIterationCondition for AllRules {
    fn process_import(
        _: &SharedRwLockReadGuard,
        _: &Device,
        _: QuirksMode,
        _: &CustomMediaMap,
        _: &ImportRule,
    ) -> bool {
        true
    }

    fn process_media(
        _: &SharedRwLockReadGuard,
        _: &Device,
        _: QuirksMode,
        _: &CustomMediaMap,
        _: &MediaRule,
    ) -> bool {
        true
    }

    fn process_document(
        _: &SharedRwLockReadGuard,
        _: &Device,
        _: QuirksMode,
        _: &DocumentRule,
    ) -> bool {
        true
    }

    fn process_supports(
        _: &SharedRwLockReadGuard,
        _: &Device,
        _: QuirksMode,
        _: &SupportsRule,
    ) -> bool {
        true
    }
}

/// An iterator over all the effective rules of a stylesheet.
///
/// NOTE: This iterator recurses into `@import` rules.
pub type EffectiveRulesIterator<'a, 'b, CMM> = RulesIterator<'a, 'b, EffectiveRules, CMM>;

impl<'a, 'b, CMM> EffectiveRulesIterator<'a, 'b, CMM>
where
    CMM: Deref<Target = CustomMediaMap>,
{
    /// Returns an iterator over the effective children of a rule, even if
    /// `rule` itself is not effective.
    pub fn effective_children(
        device: &'a Device,
        quirks_mode: QuirksMode,
        custom_media_map: CMM,
        guard: &'a SharedRwLockReadGuard<'b>,
        rule: &'a CssRule,
    ) -> Self {
        let children = RulesIterator::<AllRules, CMM>::children(
            rule,
            device,
            quirks_mode,
            &custom_media_map,
            guard,
            &mut false,
        );
        EffectiveRulesIterator::new(
            device,
            quirks_mode,
            custom_media_map,
            guard,
            children.iter(),
        )
    }
}
