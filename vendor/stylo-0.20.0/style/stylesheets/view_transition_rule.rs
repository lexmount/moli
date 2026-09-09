/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! A [`@view-transition`][view-transition] rule
//!
//! [view-transition]: https://drafts.csswg.org/css-view-transitions-2/#view-transition-rule

use crate::derives::*;
use crate::error_reporting::ContextualParseError;
use crate::parser::ParserContext;
use crate::properties::view_transition::{DescriptorParser, Descriptors};
use crate::shared_lock::DeepCloneWithLock;
use crate::shared_lock::{SharedRwLock, SharedRwLockReadGuard, ToCssWithGuard};
use cssparser::{Parser, RuleBodyParser, SourceLocation};
use std::fmt::{self, Write};
use style_traits::{CssStringWriter, CssWriter, ToCss};

/// A view-transition rule
#[derive(Clone, Debug, MallocSizeOf, ToShmem)]
pub struct ViewTransitionRule {
    /// The descriptors of this view-transition rule.
    pub descriptors: Descriptors,
    /// The source position where this view-transition rule was found.
    pub source_location: SourceLocation,
}

impl ViewTransitionRule {
    /// Parses a ViewTransitionRule
    pub fn parse(context: &ParserContext, input: &mut Parser, location: SourceLocation) -> Self {
        let mut rule = ViewTransitionRule {
            descriptors: Descriptors::default(),
            source_location: location,
        };

        let mut parser = DescriptorParser {
            context,
            descriptors: &mut rule.descriptors,
        };
        let mut iter = RuleBodyParser::new(input, &mut parser);

        while let Some(declaration) = iter.next() {
            if let Err((error, slice)) = declaration {
                let location = error.location;
                let error = ContextualParseError::UnsupportedViewTransitionDescriptor(slice, error);
                context.log_css_error(location, error);
            }
        }

        rule
    }
}

impl ToCssWithGuard for ViewTransitionRule {
    fn to_css(&self, _guard: &SharedRwLockReadGuard, dest: &mut CssStringWriter) -> fmt::Result {
        dest.write_str("@view-transition { ")?;
        self.descriptors.to_css(&mut CssWriter::new(dest))?;
        dest.write_str("}")
    }
}

impl DeepCloneWithLock for ViewTransitionRule {
    fn deep_clone_with_lock(&self, _lock: &SharedRwLock, _guard: &SharedRwLockReadGuard) -> Self {
        self.clone()
    }
}

/// <https://drafts.csswg.org/css-view-transitions-2/#view-transition-navigation>
#[derive(Clone, Copy, Debug, Default, MallocSizeOf, Parse, PartialEq, ToCss, ToShmem)]
#[repr(u8)]
pub enum NavigationType {
    /// No transition
    #[default]
    None,
    /// Perform transition if conditions are met
    Auto,
}

/// The `types` descriptor value: `none | <custom-ident>+`
///
/// <https://drafts.csswg.org/css-view-transitions-2/#typedef-pt-name-selector-list>
pub use crate::values::specified::animation::ViewTransitionClass as TransitionTypes;
